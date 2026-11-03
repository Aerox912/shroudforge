local overrides = runtime.overrides.new()
local pitch_overrides = runtime.overrides.new()
local active_session
local last_state, last_detail
local gliders = {}
local peak_flight_delta = 0
local upward_recovery = false
local flight_feedback
local configured_flight_setting
local effective_flight_setting
local token_sequence = 0
local transport_session
local server_epoch
local server_last_sequences = {}
local client_transport = {}
local native_peer_intents = {}
local clear_native_peer_intents

local item_id_path = "currentAction.triggerContext.usedItem.itemId"
local pitch_path = "equipment.gliderConfig.pitchAngleSpeed"
local glider_fields = {
    {setting="turnSpeed", path="equipment.gliderConfig.yawAngleSpeed"},
    {setting="moveSpeed", path="equipment.gliderConfig.accelerationForward"},
    {setting="liftSpeed", path="equipment.gliderConfig.airResistanceLongitudinal"},
    {setting="descendSpeed", path="equipment.gliderConfig.airResistanceVertical"},
}

local function report(state, detail)
    if flight_feedback then
        detail = (detail and detail ~= "" and detail .. " | " or "") .. flight_feedback
    end
    if last_state == state and last_detail == detail then return end
    last_state, last_detail = state, detail
    runtime.report_effect(state, detail)
end

local function finite(value)
    return type(value) == "number" and value == value and math.abs(value) < math.huge
end

local function show_flight_notice(message)
    if runtime.is_server == true or type(shroudforge) ~= "table" then return end
    local mod_runtime = shroudforge.runtime
    local notice = type(mod_runtime) == "table" and mod_runtime.notice
    if type(notice) == "table" and type(notice.show) == "function" then
        -- A missing/unavailable renderer must not change F6 or retry each tick.
        pcall(notice.show, message)
    end
end

local function queue_flight_feedback(message, notice)
    flight_feedback = message
    if notice then show_flight_notice(notice) end
end

local function configured_flight_enabled()
    return shroudforge.settings.get("unlimitedFlying", false) == true
end

local function flight_enabled()
    local configured = configured_flight_enabled()
    if configured_flight_setting == nil then
        configured_flight_setting, effective_flight_setting = configured, configured
    elseif configured_flight_setting ~= configured then
        -- A saved Modloader change remains authoritative over the temporary F6
        -- runtime toggle and clears that toggle override.
        configured_flight_setting, effective_flight_setting = configured, configured
    end
    return effective_flight_setting == true
end

local function setting_enabled(key)
    if key == "unlimitedFlying" then return flight_enabled() end
    if key == "unlimitedGliderStamina" then
        return shroudforge.settings.get(key, false) == true
    end
    local value = shroudforge.settings.get(key, 100)
    return finite(value) and value ~= 100
end

local function valid_steam_id(value)
    return type(value) == "string" and #value >= 16 and #value <= 20
        and value ~= "0" and value:match("^%d+$") ~= nil
end

local function valid_token(value)
    return type(value) == "string" and #value >= 1 and #value <= 64
        and value:match("^[%w_%-]+$") ~= nil
end

local function valid_sequence(value)
    local sequence = tonumber(value)
    return finite(sequence) and sequence >= 1 and sequence <= 2147483647
        and sequence % 1 == 0
end

local function new_token()
    token_sequence = token_sequence + 1
    local identity = tostring({}):match("0x(%x+)") or "sf"
    local session = runtime.world.session_id()
    local random = math.random(0, 2147483647)
    return table.concat({identity, tostring(session), tostring(token_sequence), string.format("%08x", random)}, "-")
end

local function reset_transport_session(session)
    if clear_native_peer_intents then
        local cleared, reason = clear_native_peer_intents()
        if not cleared then error("Native Flight cleanup pending: " .. tostring(reason)) end
    end
    transport_session = session
    server_epoch = new_token()
    server_last_sequences = {}
    flight_feedback = nil
    client_transport.peer = nil
    client_transport.nonce = nil
    client_transport.epoch = nil
    client_transport.next_sequence = nil
    client_transport.pending_state = nil
    client_transport.latest_sequence = nil
    client_transport.latest_state = nil
    client_transport.request_nonce = nil
    client_transport.awaiting_hello = false
    client_transport.awaiting_result = false
    client_transport.hello_elapsed = 1
    client_transport.send_elapsed = 1
    client_transport.request_elapsed = 0
end

local function network_status()
    if type(runtime.network) ~= "table" or type(runtime.network.status) ~= "function" then return nil end
    local ok, status = pcall(runtime.network.status)
    if not ok or type(status) ~= "table" then return nil end
    return status
end

local function current_server_peer()
    if runtime.is_server == true then return nil end
    local status = network_status()
    if not status or status.available ~= true or status.role == "server" then return nil end
    local candidates = {
        status.local_dedicated_server_steam_id,
        status.remote_dedicated_server_steam_id,
        status.configured_server_steam_id,
    }
    for index = 1, 3 do
        if valid_steam_id(candidates[index]) then return candidates[index] end
    end
    return nil
end

local function send_mod_message(peer, payload)
    if not valid_steam_id(peer) or type(runtime.network) ~= "table"
        or type(runtime.network.send_mod) ~= "function" then
        return false
    end
    local ok, sent = pcall(runtime.network.send_mod, peer, "sf-unlimited-flight", payload, {reliable=true})
    return ok and sent == true
end

local function prepare_client_peer()
    local peer = current_server_peer()
    if peer == client_transport.peer then return end
    client_transport.peer = peer
    client_transport.nonce = peer and new_token() or nil
    client_transport.epoch = nil
    client_transport.next_sequence = nil
    client_transport.pending_state = nil
    client_transport.latest_sequence = nil
    client_transport.latest_state = nil
    client_transport.request_nonce = nil
    client_transport.awaiting_hello = false
    client_transport.awaiting_result = false
    client_transport.hello_elapsed = 1
    client_transport.send_elapsed = 1
    client_transport.request_elapsed = 0
end

local function toggle_flight_from_hotkey()
    if runtime.is_server == true or type(shroudforge) ~= "table"
        or type(shroudforge.input) ~= "table"
        or type(shroudforge.input.was_keybind_pressed) ~= "function" then
        return
    end
    local ok, pressed = pcall(shroudforge.input.was_keybind_pressed, "toggleUpwardGliding")
    if not ok or pressed ~= true then return end
    effective_flight_setting = not flight_enabled()
    local desired = effective_flight_setting and "ON" or "OFF"
    local notice_state = effective_flight_setting and "On" or "Off"
    if client_transport.peer then
        client_transport.pending_state = effective_flight_setting
        queue_flight_feedback("F6 selected Upward Gliding " .. desired .. " locally. The Dedicated Server has not confirmed a player effect.",
            "Unlimited Glider Flight " .. notice_state .. " requested; server unconfirmed")
    else
        queue_flight_feedback("F6 selected Upward Gliding " .. desired .. " for this local runtime. Check the effect status for confirmation.",
            "Unlimited Glider Flight " .. notice_state .. " (local setting)")
    end
end

local function connected_peer_set()
    if type(runtime.network) ~= "table" or type(runtime.network.connected_peers) ~= "function" then return nil end
    local ok, peers = pcall(runtime.network.connected_peers)
    if not ok or type(peers) ~= "table" then return nil end
    local result = {}
    for _, peer in ipairs(peers) do
        if valid_steam_id(peer) then result[peer] = true end
    end
    return result
end

local function server_reply(peer, payload)
    send_mod_message(peer, payload)
end

local function native_peer_control(peer, enabled)
    local setter = type(runtime.ecs) == "table" and runtime.ecs.set_peer_flight
    if type(setter) ~= "function" then return false, "native per-player Flight unsupported" end
    local ok, accepted, reason = pcall(setter, peer, enabled)
    if not ok then return false, "native Flight control fault", true end
    if accepted ~= true then return false, tostring(reason or "native per-player Flight unavailable") end
    return true
end

clear_native_peer_intents = function(peers)
    local first_reason
    for peer in pairs(native_peer_intents) do
        if not peers or not peers[peer] then
            local cleared, reason = native_peer_control(peer, false)
            if cleared then native_peer_intents[peer] = nil else first_reason = first_reason or reason end
        end
    end
    return first_reason == nil, first_reason
end

local function handle_server_message(message, peers)
    if type(message) ~= "table" or type(peers) ~= "table" then return end
    local peer, payload = message.peer_steam_id, message.payload
    if not valid_steam_id(peer) or not peers[peer] or message.reliable ~= true
        or message.from_mod ~= "sf-unlimited-flight" or message.to_mod ~= "sf-unlimited-flight"
        or type(payload) ~= "string" or #payload > 256 then
        return
    end

    local nonce = payload:match("^H|1|([%w_%-]+)$")
    if valid_token(nonce) then
        local next_sequence = (server_last_sequences[peer] or 0) + 1
        if valid_sequence(next_sequence) then
            server_reply(peer, table.concat({"H", "1", nonce, server_epoch, next_sequence}, "|"))
        end
        return
    end

    local request_nonce, request_epoch, sequence_text, desired =
        payload:match("^S|1|([%w_%-]+)|([%w_%-]+)|(%d+)|([01])$")
    local sequence = tonumber(sequence_text)
    if not valid_token(request_nonce) or not valid_token(request_epoch)
        or not valid_sequence(sequence) or (desired ~= "0" and desired ~= "1") then
        return
    end
    if request_epoch ~= server_epoch then
        server_reply(peer, table.concat({"R", "1", request_nonce, server_epoch, sequence,
            "rejected", "stale_server_session"}, "|"))
        return
    end
    local last_sequence = server_last_sequences[peer] or 0
    if sequence <= last_sequence then
        server_reply(peer, table.concat({"R", "1", request_nonce, server_epoch, sequence,
            "rejected", "stale_request"}, "|"))
        return
    end
    if sequence ~= last_sequence + 1 then
        server_reply(peer, table.concat({"R", "1", request_nonce, server_epoch, sequence,
            "rejected", "out_of_order_request"}, "|"))
        return
    end

    server_last_sequences[peer] = sequence
    if type(runtime.ecs) == "table" and type(runtime.ecs.set_peer_flight) == "function" then
        -- Record potential ownership before calling native code. Even a Lua
        -- error after native acceptance must leave a peer for unload to clear.
        local was_tracked = native_peer_intents[peer] ~= nil
        if not was_tracked then native_peer_intents[peer] = false end
        local accepted, reason, uncertain = native_peer_control(peer, desired == "1")
        if not accepted and not was_tracked and not uncertain then native_peer_intents[peer] = nil end
        if accepted then
            native_peer_intents[peer] = desired == "1" or nil
            server_reply(peer, table.concat({"R", "1", request_nonce, server_epoch, sequence,
                "accepted", desired == "1" and "native_flight_on" or "native_flight_off"}, "|"))
        elseif reason:find("busy", 1, true) then
            server_reply(peer, table.concat({"R", "1", request_nonce, server_epoch, sequence,
                "pending", "native_flight_busy"}, "|"))
        elseif reason:find("fault", 1, true) then
            local cleared = native_peer_control(peer, false)
            if cleared then native_peer_intents[peer] = nil end
            server_reply(peer, table.concat({"R", "1", request_nonce, server_epoch, sequence,
                "rejected", "native_flight_fault"}, "|"))
        else
            server_reply(peer, table.concat({"R", "1", request_nonce, server_epoch, sequence,
                "unsupported", "native_flight_unavailable"}, "|"))
        end
        return
    end
    -- Older providers retain the explicit refusal. Transport delivery alone
    -- does not establish a current player mapping or native Flight effect.
    server_reply(peer, table.concat({"R", "1", request_nonce, server_epoch, sequence,
        "unsupported", "authoritative_requester_mapping_unavailable"}, "|"))
end

local function handle_client_message(message)
    if type(message) ~= "table" or not valid_steam_id(client_transport.peer)
        or message.peer_steam_id ~= client_transport.peer or message.reliable ~= true
        or message.from_mod ~= "sf-unlimited-flight" or message.to_mod ~= "sf-unlimited-flight"
        or type(message.payload) ~= "string" or #message.payload > 256 then
        return
    end

    local nonce, epoch, next_sequence_text =
        message.payload:match("^H|1|([%w_%-]+)|([%w_%-]+)|(%d+)$")
    local next_sequence = tonumber(next_sequence_text)
    if client_transport.awaiting_hello and valid_token(nonce) and nonce == client_transport.nonce
        and valid_token(epoch) and valid_sequence(next_sequence) then
        client_transport.epoch = epoch
        client_transport.next_sequence = next_sequence
        client_transport.awaiting_hello = false
        client_transport.hello_elapsed = 0
        return
    end

    local result_nonce, result_epoch, sequence_text, result, reason =
        message.payload:match("^R|1|([%w_%-]+)|([%w_%-]+)|(%d+)|([%a]+)|([%w_%-]+)$")
    local sequence = tonumber(sequence_text)
    if not valid_token(result_nonce) or result_nonce ~= client_transport.request_nonce
        or not valid_token(result_epoch) or not valid_sequence(sequence)
        or sequence ~= client_transport.latest_sequence then
        return
    end
    if result == "rejected" and reason == "stale_server_session" then
        client_transport.pending_state = client_transport.latest_state
        client_transport.latest_sequence = nil
        client_transport.awaiting_result = false
        client_transport.epoch = nil
        client_transport.next_sequence = nil
        client_transport.nonce = new_token()
        client_transport.request_nonce = nil
        client_transport.awaiting_hello = false
        client_transport.hello_elapsed = 1
        queue_flight_feedback("The Dedicated Server rejected a stale Flight session. The current local preference is unconfirmed.",
            "Unlimited Glider Flight unconfirmed: server session changed")
        return
    end
    if result_epoch ~= client_transport.epoch then return end
    client_transport.latest_sequence = nil
    client_transport.awaiting_result = false
    if result == "accepted" and (reason == "native_flight_on" or reason == "native_flight_off") then
        queue_flight_feedback("The Dedicated Server accepted this player's Flight " ..
            (reason == "native_flight_on" and "On" or "Off") .. " intent. Movement remains unconfirmed.")
    elseif result == "pending" and reason == "native_flight_busy" then
        client_transport.pending_state = client_transport.latest_state
        queue_flight_feedback("The Dedicated Server is busy. Retrying this player's unconfirmed Flight preference.")
    elseif result == "rejected" and reason == "native_flight_fault" then
        queue_flight_feedback("The Dedicated Server could not confirm native Flight state. Toggle Off to clear this player's intent.",
            "Unlimited Glider Flight unconfirmed: server error")
    elseif result == "unsupported" and reason == "native_flight_unavailable" then
        queue_flight_feedback("The current server provider cannot accept native per-player Flight.",
            "Unlimited Glider Flight unavailable on this server")
    elseif result == "unsupported" and reason == "authoritative_requester_mapping_unavailable" then
        queue_flight_feedback("Dedicated Server response: unsupported. It cannot map the authenticated requester to the authoritative player entity. No server Flight state was changed.",
            "Unlimited Glider Flight unavailable: server player mapping unsupported")
    elseif result == "rejected" then
        queue_flight_feedback("The Dedicated Server rejected an outdated or out-of-order Flight request. No server Flight state was changed.",
            "Unlimited Glider Flight request rejected; server state unchanged")
    end
end

local function update_client_transport(delta_seconds)
    if not client_transport.peer then return end
    local delta = finite(delta_seconds) and math.max(0, delta_seconds) or 0
    client_transport.hello_elapsed = (client_transport.hello_elapsed or 0) + delta
    client_transport.send_elapsed = (client_transport.send_elapsed or 0) + delta
    if client_transport.awaiting_result then
        client_transport.request_elapsed = (client_transport.request_elapsed or 0) + delta
        if client_transport.request_elapsed >= 5 then
            client_transport.pending_state = client_transport.latest_state
            client_transport.latest_sequence = nil
            client_transport.awaiting_result = false
            client_transport.epoch = nil
            client_transport.next_sequence = nil
            client_transport.nonce = new_token()
            client_transport.request_nonce = nil
            client_transport.awaiting_hello = false
            client_transport.hello_elapsed = 1
            queue_flight_feedback("No Dedicated Server reply was received. The server Flight state is unknown.")
        end
    end

    if not client_transport.epoch then
        if client_transport.hello_elapsed >= 1 then
            client_transport.nonce = client_transport.nonce or new_token()
            local sent = send_mod_message(client_transport.peer,
                table.concat({"H", "1", client_transport.nonce}, "|"))
            client_transport.awaiting_hello = sent
            client_transport.hello_elapsed = 0
        end
        return
    end

    if client_transport.pending_state ~= nil and client_transport.send_elapsed >= 1 then
        local sequence = client_transport.next_sequence
        if not valid_sequence(sequence) then
            client_transport.epoch = nil
            client_transport.next_sequence = nil
            client_transport.awaiting_hello = false
            client_transport.hello_elapsed = 1
            return
        end
        local desired = client_transport.pending_state and "1" or "0"
        local payload = table.concat({"S", "1", client_transport.nonce,
            client_transport.epoch, sequence, desired}, "|")
        local sent = send_mod_message(client_transport.peer, payload)
        client_transport.send_elapsed = 0
        if sent then
            client_transport.pending_state = nil
            client_transport.latest_sequence = sequence
            client_transport.latest_state = desired == "1"
            client_transport.request_nonce = client_transport.nonce
            client_transport.next_sequence = sequence + 1
            client_transport.awaiting_result = true
            client_transport.request_elapsed = 0
        end
    end
end

local function update_network(delta_seconds)
    if type(runtime.network) ~= "table" or type(runtime.network.receive_mod) ~= "function" then return end
    local ok, messages = pcall(runtime.network.receive_mod, 16)
    if ok and type(messages) == "table" then
        if runtime.is_server == true then
            local peers = connected_peer_set()
            clear_native_peer_intents(peers)
            if peers then
                for _, message in ipairs(messages) do handle_server_message(message, peers) end
            end
        else
            for _, message in ipairs(messages) do handle_client_message(message) end
        end
    end
    if runtime.is_server ~= true then update_client_transport(delta_seconds) end
end

local function reset_flight_toggle()
    configured_flight_setting, effective_flight_setting = nil, nil
end

local function multiplier(key)
    local value = shroudforge.settings.get(key, 100)
    if not finite(value) then return nil, "setting must be a finite percentage" end
    if value < 25 or value > 400 then return nil, "setting percentage must be between 25 and 400" end
    return value / 100
end

local function has_effect()
    if setting_enabled("unlimitedFlying") or setting_enabled("unlimitedGliderStamina") then return true end
    for _, field in ipairs(glider_fields) do
        if setting_enabled(field.setting) then return true end
    end
    return false
end

local function resource_identity(resource)
    return {guid=resource.guid, part=resource.part}
end

local function validate_glider(resource, item_id)
    if not resource or resource.loaded ~= true then return nil, "ItemInfo is not a loaded runtime resource" end
    local ok, data = pcall(function() return resource.data end)
    if not ok then return nil, tostring(data) end
    local resource_item_id = data and data.itemId
    if type(resource_item_id) == "table" or type(resource_item_id) == "userdata" then
        resource_item_id = resource_item_id.value
    end
    if not data or resource_item_id ~= item_id or not data.equipment or data.equipment.slot ~= "Glider" then
        return nil, "The active item is not a registered glider"
    end
    return data
end

local function current_glider(item_id)
    local key = tostring(item_id)
    if type(game.assets.resolve_loaded_resource) == "function" then
        local ok, resource, reason = pcall(game.assets.resolve_loaded_resource,
            "g38_items::ItemRegistry", item_id)
        if not ok then return nil, tostring(resource) end
        if resource then
            local data, failure = validate_glider(resource, item_id)
            if not data then return nil, failure end
            gliders[key] = resource_identity(resource)
            return resource
        end
        -- Older exact profiles may lack the relation extension. Those keep
        -- the common prepared index as their explicit compatible fallback.
        reason = tostring(reason or "Original item registry is unavailable")
        if not reason:find("lacks", 1, true) and not reason:find("contract is unavailable", 1, true) then
            return nil, reason
        end
    end

    -- Only the compatibility path retains an identity hint. The canonical engine
    -- relation is rechecked on every supported lookup, including map remaps.
    local identity = gliders[key]
    if identity then
        local ok, resource = pcall(game.assets.get_loaded_resource,
            identity.guid, "keen::ItemInfo", identity.part)
        if ok and resource then
            local data = validate_glider(resource, item_id)
            if data then return resource end
        end
        gliders[key] = nil
    end

    local ok, resource, reason, progress = pcall(game.assets.get_loaded_resource_by_field,
        "keen::ItemInfo", "itemId.value", item_id)
    if not ok then return nil, tostring(resource) end
    if not resource then
        if progress then
            return nil, tostring(reason or (progress.phase .. " " .. progress.scanned .. "/" .. progress.total))
        end
        return nil, tostring(reason or "Active glider resource is unavailable")
    end
    local data, failure = validate_glider(resource, item_id)
    if not data then return nil, failure end
    gliders[key] = resource_identity(resource)
    return resource
end

local function active_item(entity)
    local actor, reason = runtime.ecs.read_fields(entity, "keen::ecs::Actor", {
        item_id_path, "currentState", "currentAction.playbackContext.state",
        "currentAction.playbackContext.setStateMask", "currentAction.playbackContext.unsetStateMask"})
    if not actor then return nil, reason end
    local function has_flag(flags, name)
        if flags == nil then return false end
        for _, flag in ipairs(flags) do if flag == name then return true end end
        return false
    end
    -- The original state getter combines the active action's state masks.
    -- HangGliding need not exist in the stored currentState by itself.
    local active = has_flag(actor["currentAction.playbackContext.state"], "IsActive")
    local gliding = has_flag(actor.currentState, "HangGliding")
    if active then
        gliding = (gliding or has_flag(actor["currentAction.playbackContext.setStateMask"], "HangGliding"))
            and not has_flag(actor["currentAction.playbackContext.unsetStateMask"], "HangGliding")
    end
    if not gliding or not active then
        return nil, "Player is not actively using a glider"
    end
    local item_id = actor[item_id_path]
    if type(item_id) == "table" or type(item_id) == "userdata" then item_id = item_id.value end
    if not finite(item_id) or item_id <= 0 then return nil, "Player is not actively using a glider" end
    return item_id
end

local function pitch_command(entity, delta_seconds)
    if not finite(delta_seconds) or delta_seconds <= 0 then
        return nil, "Glider steering requires a positive measured update interval"
    end
    local mode, reason = runtime.ecs.read_fields(entity, "keen::ecs::PlayerInput",
        {"fromClient.movementStickInputMode"})
    if not mode then return nil, reason end
    local value = mode["fromClient.movementStickInputMode"]
    if value ~= "YawSpeedPitchSpeed" and value ~= 0 then
        return nil, "Upward tilt requires pitch-speed input controls. Target-angle controls are not supported"
    end
    local input, why = runtime.ecs.read_fields(entity, "keen::ecs::ActorInput", {"desiredLocalMoveInput.z"})
    if not input then return nil, why end
    local command = input["desiredLocalMoveInput.z"]
    if not finite(command) or math.abs(command) > 1.001 then return nil, "Glider pitch input is unavailable" end
    command = math.max(-1, math.min(1, command))
    local prefix = "transform.orientation."
    local rotation, failure = runtime.ecs.read_fields(entity, "keen::ecs::CurrentTransform",
        {prefix .. "x", prefix .. "y", prefix .. "z", prefix .. "w"})
    if not rotation then return nil, failure end
    local gravity, error = runtime.ecs.read_fields(entity, "keen::ecs::Gravity",
        {"direction.x", "direction.y", "direction.z"})
    if not gravity then return nil, error end
    local x, y, z, w = rotation[prefix .. "x"], rotation[prefix .. "y"], rotation[prefix .. "z"], rotation[prefix .. "w"]
    local gx, gy, gz = gravity["direction.x"], gravity["direction.y"], gravity["direction.z"]
    if not (finite(x) and finite(y) and finite(z) and finite(w)
        and finite(gx) and finite(gy) and finite(gz)) then return nil, "Glider orientation or gravity is invalid" end
    local norm, length = x*x + y*y + z*z + w*w, math.sqrt(gx*gx + gy*gy + gz*gz)
    if norm < 0.99 or norm > 1.01 or length < 0.001 then return nil, "Glider orientation or gravity is not normalized" end
    local forward_x = 2 * (x*z + w*y) / norm
    local forward_y = 2 * (y*z - w*x) / norm
    local forward_z = 1 - 2 * (x*x + y*y) / norm
    local pitch = math.deg(math.asin(math.max(-1, math.min(1,
        (forward_x*gx + forward_y*gy + forward_z*gz) / length))))
    local up_x = 2 * (x*y - w*z) / norm
    local up_y = 1 - 2 * (x*x + z*z) / norm
    local up_z = 2 * (y*z + w*x) / norm
    local upright = -(up_x*gx + up_y*gy + up_z*gz) / length
    -- asin folds back into [-90,90] after crossing the vertical. Keep that
    -- folded angle from restarting a negative rate while climb stays held.
    if upright <= 0 then upward_recovery = true end
    if upright > 0 and command >= 0 then upward_recovery = false end
    peak_flight_delta = math.max(peak_flight_delta, delta_seconds)
    local horizon = math.max(0.25, 2 * peak_flight_delta)
    -- This margin uses measured runtime intervals. A resource rate has no
    -- expiry, so an arbitrarily longer future Lua pause is still unbounded.
    return {input=command, recovery=upward_recovery,
        upward_rate_limit=math.max(0, (pitch + 85) / horizon)}
end

local function update(delta_seconds)
    local session = runtime.world.session_id()
    if session == 0 then
        peak_flight_delta, upward_recovery = 0, false
        flight_feedback = nil
        report("waiting", "Waiting for an authoritative world") return
    end
    if active_session and session ~= active_session then
        gliders = {}
        peak_flight_delta, upward_recovery = 0, false
        reset_flight_toggle()
    end
    if transport_session ~= session then reset_transport_session(session) end
    active_session = session
    if runtime.is_server ~= true then
        prepare_client_peer()
        toggle_flight_from_hotkey()
    end
    update_network(delta_seconds)
    overrides:begin_update()
    pitch_overrides:begin_update()

    overrides:restore_disabled(setting_enabled)
    pitch_overrides:restore_disabled(setting_enabled)
    if not setting_enabled("unlimitedFlying") then
        peak_flight_delta, upward_recovery = 0, false
    end
    if overrides.fault then report("write-failed", overrides.fault) return end
    if pitch_overrides.fault then report("write-failed", pitch_overrides.fault) return end
    if runtime.is_server == true then
        -- This candidate's server effect is exclusively the native actor
        -- operand. Never reintroduce global pitch-speed or stamina writes.
        pitch_overrides:restore_all()
        overrides:restore_all()
        report("intent-only", "Per-player Flight controls; stamina unchanged. Native movement requires gameplay verification")
        return
    end
    if not has_effect() then
        report(overrides:has_ownership() and "waiting" or "no-change",
            overrides:summary("Unlimited Flying and all speed multipliers are at their defaults"))
        return
    end
    if not runtime.has("runtime.ecs.read") or not runtime.has("runtime.ecs.write") then
        report("waiting", "Waiting for typed ECS access and the authoritative write scope")
        return
    end

    local players, reason = runtime.ecs.query("keen::ecs::PlayerInput")
    if not players then
        pitch_overrides:restore_all()
        report("waiting", tostring(reason or "Player query is pending")) return
    end
    if #players == 0 then
        peak_flight_delta, upward_recovery = 0, false
        pitch_overrides:restore_all()
        report("no-target", "No live player entity is available") return
    end

    local pitch_detail = ""
    for _, entity in ipairs(players) do
        if setting_enabled("unlimitedGliderStamina") then
            local target, failure = runtime.overrides.attribute(entity, "StaminaDepletionMod_Glider")
            if target then
                overrides:apply(target.key, "unlimitedGliderStamina", target, function() return 0 end)
            else overrides:waiting("Glider stamina: " .. tostring(failure)) end
        end

        local item_id, item_reason = active_item(entity)
        if item_id then
            local resource, resource_reason = current_glider(item_id)
            if resource then
                if setting_enabled("unlimitedFlying") then
                    if #players == 1 then
                        local command, failure = pitch_command(entity, delta_seconds)
                        if command then
                            if command.recovery then
                                pitch_detail = " | inverted recovery: release climb to resume"
                            end
                            local target, why = runtime.overrides.resource(resource, pitch_path)
                            if target then
                                pitch_overrides:apply(target.key, "unlimitedFlying", target, function(_, baseline)
                                    if not finite(baseline) or baseline <= 0 then return nil, "A positive original pitch rate is required" end
                                    -- A positive original rate steers back toward
                                    -- the engine's bounded native target.
                                    if command.recovery then return baseline end
                                    if command.input < 0 then return -math.min(baseline * -command.input, command.upward_rate_limit) end
                                    if command.input == 0 then return 0 end
                                    return baseline
                                end)
                            else pitch_overrides:waiting(tostring(why)) end
                        else pitch_overrides:waiting(tostring(failure)) end
                    else pitch_overrides:waiting("Upward tilt requires one authoritative player because glider ItemInfo is shared") end
                end
                for _, field in ipairs(glider_fields) do
                    if setting_enabled(field.setting) then
                        local factor, why = multiplier(field.setting)
                        if factor then
                            -- Toolkit values are direct multipliers of these original coefficients.
                            -- The adapter captures a fresh snapshot before each field apply.
                            local target, failure = runtime.overrides.resource(resource, field.path)
                            if target then
                                overrides:apply(target.key, field.setting, target, function(snapshot, baseline)
                                    return baseline * factor
                                end)
                            else overrides:waiting(field.setting .. ": " .. tostring(failure)) end
                        else
                            overrides:waiting(field.setting .. ": " .. tostring(why))
                        end
                    end
                end
            elseif setting_enabled("unlimitedFlying") or setting_enabled("turnSpeed") or setting_enabled("moveSpeed")
                or setting_enabled("liftSpeed") or setting_enabled("descendSpeed") then
                overrides:waiting("Glider settings: " .. tostring(resource_reason))
            end
        elseif setting_enabled("unlimitedFlying") or setting_enabled("turnSpeed") or setting_enabled("moveSpeed")
            or setting_enabled("liftSpeed") or setting_enabled("descendSpeed") then
            peak_flight_delta, upward_recovery = 0, false
            overrides:waiting("Glider settings: " .. tostring(item_reason))
        end
    end

    pitch_overrides:restore_unused()

    local state = (overrides.fault or pitch_overrides.fault) and "write-failed" or
        (overrides.waiting_count + pitch_overrides.waiting_count > 0 and "waiting" or
        (overrides.writes + pitch_overrides.writes > 0 and "write-confirmed" or "no-change"))
    report(state, pitch_overrides:summary(overrides:summary("Native glider controls")) .. pitch_detail)
end

return {
    update_interval_ms = 100,
    on_load = function()
        reset_transport_session(runtime.world.session_id())
        if type(game.assets.prepare_loaded_index) == "function" then
            game.assets.prepare_loaded_index("keen::ItemInfo", "itemId.value")
        end
        report("waiting", "SF Unlimited Flight loaded. Waiting for typed engine access")
    end,
    on_update = update,
    on_unload = function()
        local cleared, reason = clear_native_peer_intents()
        if not cleared then error("Native Flight cleanup pending: " .. tostring(reason)) end
    end,
}
