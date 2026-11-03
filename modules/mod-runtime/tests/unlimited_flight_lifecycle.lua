local function equal(actual, expected, label)
    assert(actual == expected, (label or "value") .. ": expected " .. tostring(expected) .. ", got " .. tostring(actual))
end

local function clone(value)
    if type(value) ~= "table" then return value end
    local copy = {}
    for key, item in pairs(value) do copy[key] = clone(item) end
    return copy
end

local function get_path(root, path)
    local value = root
    for part in path:gmatch("[^%.]+") do value = value[part] end
    return value
end

local function set_path(root, path, value)
    local parent = root
    local parts = {}
    for part in path:gmatch("[^%.]+") do parts[#parts + 1] = part end
    for i = 1, #parts - 1 do parent = parent[parts[i]] end
    parent[parts[#parts]] = value
end

local function fixture()
    local f = {
        settings={}, session=41, writes=0, players={11}, active_item=0,
        pitch_mode="YawSpeedPitchTargetAngle", pitch_input=0, pitch_degrees=0, delta_seconds=1/60,
        server=false, key_pressed=false, remote_server=nil, incoming={}, outgoing={}, connected_peers={},
        action_sets_gliding=false, action_unsets_gliding=false,
        attributes={[11]={StaminaDepletionMod_Glider=0.35}},
        trackers={},
        item_id=3357517884,
        glider={
            itemId={value=3357517884},
            equipment={slot="Glider",gliderConfig={
                yawAngleSpeed=55,
                pitchAngleSpeed=70,
                accelerationForward=1,
                airResistanceLongitudinal=string.unpack("f", string.pack("f", 0.155)),
                airResistanceVertical=3,
            }},
        },
    }
    local item_type = {qualified_name="keen::ItemInfo"}
    local scalar_type = {qualified_name="keen::float32"}
    local function resource()
        return {
            guid="80c9eaeb-7fa0-480d-a55f-91c2c51b383e",
            type=item_type, part=0, loaded=true, session_id=f.session, data=clone(f.glider),
        }
    end
    local overrides = assert(load(overrides_source, "runtime.overrides"))()
    local new_tracker = overrides.new
    overrides.new = function()
        local tracker = new_tracker(function() return f.session end)
        f.trackers[#f.trackers + 1] = tracker
        return tracker
    end
    local function claim()
        local bound_session = f.session
        return {
            acquire=function()
                if f.session ~= bound_session then return false, "claim belongs to another session" end
                return true
            end,
            release=function() end,
        }
    end
    overrides.get_component_claim = claim
    overrides.get_resource_claim = claim
    shroudforge = {
        settings={get=function(key, fallback)
            local value = f.settings[key]
            if value == nil then return fallback end
            return value
        end},
        input={was_keybind_pressed=function(key)
            equal(key, "toggleUpwardGliding", "F6 setting key")
            local pressed = f.key_pressed
            f.key_pressed = false
            return pressed
        end},
    }
    runtime = {
        overrides=overrides,
        is_server=false,
        has=function() return true end,
        report_effect=function(state, detail) f.state, f.detail = state, detail end,
        world={session_id=function() return f.session end},
        network={
            status=function()
                return {available=true, role=f.server and "server" or "client",
                    local_steam_id="76561198000000002",
                    remote_dedicated_server_steam_id=f.remote_server}
            end,
            send_mod=function(peer, target_mod, payload, options)
                f.outgoing[#f.outgoing + 1] = {peer_steam_id=peer, to_mod=target_mod,
                    from_mod="sf-unlimited-flight", payload=payload,
                    reliable=options and options.reliable}
                return true
            end,
            receive_mod=function()
                local messages = f.incoming
                f.incoming = {}
                return messages
            end,
            connected_peers=function() return f.connected_peers end,
        },
        values={
            encode=function(_, value) return string.pack("f", value) end,
            decode=function(_, bytes) return string.unpack("f", bytes) end,
            normalize_field=function(_, _, value) return string.unpack("f", string.pack("f", value)) end,
        },
        ecs={
            query=function(name)
                equal(name, "keen::ecs::PlayerInput")
                return f.players
            end,
            get_attribute=function(selector) return {name=selector, root_hash=1} end,
            read_attributes=function(entity, selector)
                equal(selector, "StaminaDepletionMod_Glider")
                return {
                    entity=entity, root_hash=1, values=clone(f.attributes[entity]),
                    component={qualified_name="keen::ecs::StaminaDepletionMod"},
                }
            end,
            evaluate_attributes=function(field, values)
                equal(field, "StaminaDepletionMod_Glider")
                return {values=clone(values)}
            end,
            update_attribute=function(entity, field, value, expected)
                equal(type(expected), "table", "attribute expected value")
                equal(f.attributes[entity][field], expected[field], "attribute compare")
                f.attributes[entity][field] = value
                f.writes = f.writes + 1
                return {
                    entity=entity, root_hash=1, values=clone(f.attributes[entity]),
                    component={qualified_name="keen::ecs::StaminaDepletionMod"},
                }
            end,
            read_fields=function(entity, type_name, paths)
                if type_name == "keen::ecs::Actor" then
                    equal(paths[1], "currentAction.triggerContext.usedItem.itemId")
                    return {[paths[1]]={value=f.active_item},
                        currentState=f.active_item > 0 and not f.action_sets_gliding and {"HangGliding"} or {},
                        ["currentAction.playbackContext.state"]=f.active_item > 0 and {"IsActive"} or {},
                        ["currentAction.playbackContext.setStateMask"]=f.action_sets_gliding and {"HangGliding"} or {},
                        ["currentAction.playbackContext.unsetStateMask"]=f.action_unsets_gliding and {"HangGliding"} or {}}
                elseif type_name == "keen::ecs::PlayerInput" then
                    return {[paths[1]]=f.pitch_mode}
                elseif type_name == "keen::ecs::ActorInput" then
                    return {[paths[1]]=f.pitch_input}
                elseif type_name == "keen::ecs::CurrentTransform" then
                    local angle = math.rad(f.pitch_degrees) / 2
                    return {[paths[1]]=math.sin(angle), [paths[2]]=0, [paths[3]]=0, [paths[4]]=math.cos(angle)}
                elseif type_name == "keen::ecs::Gravity" then
                    return {[paths[1]]=0, [paths[2]]=-1, [paths[3]]=0}
                end
                error("unexpected component " .. type_name)
            end,
        },
    }
    game = {assets={
        get_loaded_resource=function(guid, type_name, part)
            if f.read_busy then return nil, "ResourceService read gate is busy" end
            equal(guid, "80c9eaeb-7fa0-480d-a55f-91c2c51b383e")
            equal(type(type_name) == "table" and type_name.qualified_name or type_name, "keen::ItemInfo")
            equal(part, 0)
            return resource()
        end,
        get_loaded_resource_by_field=function(type_name, path, value)
            if f.read_busy then return nil, "ResourceService index gate is busy" end
            equal(type_name, "keen::ItemInfo")
            equal(path, "itemId.value")
            equal(value, f.item_id)
            return resource()
        end,
        read_loaded_field=function(snapshot, path)
            -- The returned resource owns the confirmed native snapshot.
            -- Reading it must continue to work while later live reads are busy.
            return get_path(snapshot.data, path), nil, scalar_type
        end,
        write_loaded_field=function(snapshot, path, value, expected)
            equal(snapshot.loaded, true)
            equal(get_path(f.glider, path), expected, "resource compare")
            set_path(f.glider, path, value)
            f.writes = f.writes + 1
            if f.block_after_write then f.read_busy = true end
            return resource()
        end,
    }}
    for name, target in pairs(override_targets_factory(runtime, game.assets)) do overrides[name] = target end
    f.mod = assert(load(flight_source, "flight"))()
    f.mod.on_load()
    function f:tick()
        if self.cleanup_pending then return self:unload() end
        runtime.is_server = self.server
        self.mod.on_update(self.delta_seconds)
    end
    function f:unload()
        self.cleanup_pending = true
        if self.mod.on_unload then self.mod.on_unload() end
        for _, tracker in ipairs(self.trackers) do
            tracker:begin_update()
            tracker:restore_all()
            assert(not tracker.fault, tracker.fault)
            assert(not tracker:has_ownership(), tracker:summary("API cleanup pending"))
        end
        self.cleanup_pending = false
    end
    return f
end

local function network_message(peer, payload)
    return {peer_steam_id=peer, from_mod="sf-unlimited-flight", to_mod="sf-unlimited-flight",
        payload=payload, reliable=true}
end

-- Native per-player control tests use the actual lifecycle/transport handler.
do
    local f = fixture()
    local peer = "76561198000000001"
    runtime.ecs.set_peer_flight = function() return false, "native per-player Flight unavailable" end
    f.server, f.connected_peers = true, {peer}
    f.incoming = {network_message(peer, "H|1|missing-native")}; f:tick()
    local epoch = f.outgoing[1].payload:match("^H|1|missing%-native|([%w_%-]+)|1$")
    f.incoming = {network_message(peer, "S|1|missing-native|" .. epoch .. "|1|1")}; f:tick()
    f:unload() -- A clean refusal must not invent a native cleanup obligation.
end

do
    local f = fixture()
    local a, b = "76561198000000001", "76561198000000003"
    local calls, desired = {}, {}
    runtime.ecs.set_peer_flight = function(peer, enabled)
        calls[#calls + 1] = {peer=peer, enabled=enabled}
        desired[peer] = enabled
        return true
    end
    f.server, f.connected_peers = true, {a, b}
    f.settings.unlimitedFlying, f.settings.unlimitedGliderStamina = true, true
    f.active_item = f.item_id
    f.incoming = {network_message(a, "H|1|native-a")}
    f:tick()
    local epoch = f.outgoing[1].payload:match("^H|1|native%-a|([%w_%-]+)|1$")
    assert(epoch, "native control handshake")
    f.incoming = {network_message(a, "S|1|native-a|" .. epoch .. "|1|1")}
    f:tick()
    equal(#calls, 1, "one in-order request reaches native control")
    equal(calls[1].peer, a, "identity comes from transport sender")
    equal(desired[a], true, "first player's intent On")
    assert(f.outgoing[2].payload:find("|accepted|native_flight_on$"), "acceptance describes intent")
    f.incoming = {network_message(b, "H|1|native-b")}
    f:tick()
    f.incoming = {network_message(b, "S|1|native-b|" .. epoch .. "|1|0")}
    f:tick()
    equal(desired[b], false, "second player's independent Off")
    equal(desired[a], true, "second player cannot turn off first player's intent")
    f.incoming = {network_message(a, "S|1|native-a|" .. epoch .. "|1|0")}
    f:tick()
    equal(#calls, 2, "replayed request cannot change native intent")
    equal(f.writes, 0, "native server route never writes shared resources or stamina, even with saved stamina setting")
    equal(f.attributes[11].StaminaDepletionMod_Glider, 0.35, "server stamina remains stock")
    f.connected_peers = {b}
    f:tick()
    equal(desired[a], false, "disconnect clears native intent")
    f:unload()
    equal(desired[b], false, "unload clears remaining peer intent")
end

do
    local f = fixture()
    local peer, calls, cleanup_busy = "76561198000000001", {}, true
    runtime.ecs.set_peer_flight = function(p, enabled)
        calls[#calls+1] = {peer=p, enabled=enabled}
        if not enabled and cleanup_busy then return false, "native Flight control busy" end
        return true
    end
    f.server, f.connected_peers = true, {peer}
    f.incoming = {network_message(peer, "H|1|cleanup")}; f:tick()
    local epoch = f.outgoing[1].payload:match("^H|1|cleanup|([%w_%-]+)|1$")
    f.incoming = {network_message(peer, "S|1|cleanup|" .. epoch .. "|1|1")}; f:tick()
    assert(not pcall(f.mod.on_unload), "failed native Off keeps lifecycle cleanup pending")
    cleanup_busy = false
    f:unload()
    equal(calls[#calls].enabled, false, "cleanup retries the retained peer")
end

do
    local f = fixture()
    local preparations, resolutions = 0, 0
    local read = game.assets.get_loaded_resource
    game.assets.prepare_loaded_index = function(type_name, path)
        equal(type_name, "keen::ItemInfo")
        equal(path, "itemId.value")
        preparations = preparations + 1
        return false, "queued"
    end
    f.mod.on_load()
    equal(preparations, 1, "preparation is registered before any glider action")
    game.assets.resolve_loaded_resource = function(registry_name, item_id)
        equal(registry_name, "g38_items::ItemRegistry")
        equal(item_id, f.item_id)
        resolutions = resolutions + 1
        return read("80c9eaeb-7fa0-480d-a55f-91c2c51b383e", "keen::ItemInfo", 0)
    end
    game.assets.get_loaded_resource_by_field = function()
        error("a supported canonical relation must not scan every ItemInfo")
    end
    f.active_item, f.settings.turnSpeed = f.item_id, 200
    f:tick()
    equal(f.glider.equipment.gliderConfig.yawAngleSpeed, 110, "first action uses the canonical original relation")
    f:tick()
    equal(resolutions, 2, "each canonical lookup rechecks the registry rather than an old GUID hint")
    f:unload()
end

do
    local f = fixture()
    game.assets.resolve_loaded_resource = function()
        return nil, "original ItemRegistry is busy, unavailable or changed. Retry later"
    end
    game.assets.get_loaded_resource_by_field = function()
        error("a busy validated relation must not switch to a different selection contract")
    end
    f.active_item, f.settings.turnSpeed = f.item_id, 200
    f:tick()
    equal(f.writes, 0, "a busy original relation cannot authorize a guessed resource")
    equal(f.state, "waiting")
    f:unload()
end

do
    local f = fixture()
    f.settings.unlimitedFlying, f.settings.turnSpeed = true, 200
    f.active_item, f.pitch_mode, f.pitch_input = f.item_id, "YawSpeedPitchSpeed", -1
    f.action_sets_gliding = true
    f:tick()
    equal(f.glider.equipment.gliderConfig.pitchAngleSpeed, -70, "active action can supply the effective HangGliding state")
    equal(f.glider.equipment.gliderConfig.yawAngleSpeed, 110, "effective action state enables original glider controls")
    f.action_unsets_gliding = true
    f:tick()
    equal(f.glider.equipment.gliderConfig.pitchAngleSpeed, 70, "unset mask takes precedence and restores native pitch")
    f:unload()
end

do
    local f = fixture()
    f.settings.unlimitedFlying = true
    f.active_item, f.pitch_mode, f.pitch_input = f.item_id, "YawSpeedPitchSpeed", -1
    f:tick()
    equal(f.glider.equipment.gliderConfig.pitchAngleSpeed, -70, "upward command uses the native negative rate")
    f.pitch_input = 0
    f:tick()
    equal(f.glider.equipment.gliderConfig.pitchAngleSpeed, 0, "neutral input holds the native pitch")
    f.pitch_input = 1
    f:tick()
    equal(f.glider.equipment.gliderConfig.pitchAngleSpeed, 70, "downward input restores the native positive rate")
    f.pitch_input, f.pitch_degrees = -1, -84
    f:tick()
    assert(f.glider.equipment.gliderConfig.pitchAngleSpeed > -4.01
        and f.glider.equipment.gliderConfig.pitchAngleSpeed < -3.99, "sampled tilt limit reduces the native rate")
    f.pitch_degrees = -86
    f:tick()
    equal(f.glider.equipment.gliderConfig.pitchAngleSpeed, 0, "sampled limit stops further upward steering")
    f.active_item = 0
    f:tick()
    equal(f.glider.equipment.gliderConfig.pitchAngleSpeed, 70, "landing automatically restores the native rate")
    f.active_item, f.pitch_input, f.pitch_degrees = f.item_id, -1, 0
    f:tick()
    equal(f.glider.equipment.gliderConfig.pitchAngleSpeed, -70)
    f.pitch_mode = "YawSpeedPitchTargetAngle"
    f:tick()
    equal(f.glider.equipment.gliderConfig.pitchAngleSpeed, 70, "unsupported controls restore instead of leaving a negative rate")
    f.pitch_mode = "YawSpeedPitchSpeed"
    f:tick()
    equal(f.glider.equipment.gliderConfig.pitchAngleSpeed, -70)
    f.players = {11,12}
    f.attributes[12] = {StaminaDepletionMod_Glider=0.35}
    f:tick()
    equal(f.glider.equipment.gliderConfig.pitchAngleSpeed, 70, "shared rate steering stops when another player appears")
    f:unload()
end

do
    -- This is the old controller's arithmetic counterexample, not a model of
    -- the engine's complete glider physics. asin hides crossing the pole.
    local pitch, delta = -80, 0.65
    local old_rate = -(pitch + 85) / 0.25
    local next_pitch = pitch + old_rate * delta
    assert(next_pitch < -90, "a 650 ms interval exceeds the old fixed 250 ms horizon")
    local angle = math.rad(next_pitch)
    local folded = math.deg(math.asin(math.sin(angle)))
    assert(math.cos(angle) < 0 and folded > -90,
        "after crossing vertical the folded pitch conceals the reversed horizontal heading")

    local f = fixture()
    f.settings.unlimitedFlying = true
    f.active_item, f.pitch_mode, f.pitch_input = f.item_id, "YawSpeedPitchSpeed", -1
    f.pitch_degrees, f.delta_seconds = pitch, delta
    f:tick()
    local rate = f.glider.equipment.gliderConfig.pitchAngleSpeed
    assert(math.abs(rate + 5 / 1.3) < 0.00001, "measured interval sets a two-interval steering margin")
    assert(pitch + rate * delta > -85, "the observed 650 ms interval cannot cross the 85 degree margin")
    f.delta_seconds = 1/60
    f:tick()
    equal(f.glider.equipment.gliderConfig.pitchAngleSpeed, rate, "a shorter next interval retains the active flight's peak")
    f.active_item = 0
    f:tick()
    equal(f.glider.equipment.gliderConfig.pitchAngleSpeed, 70, "landing restores the original rate after a slow interval")
    f.active_item = f.item_id
    f:tick()
    equal(f.glider.equipment.gliderConfig.pitchAngleSpeed, -20, "the next flight starts with a fresh interval history")
    f:unload()
end

do
    local f = fixture()
    f.settings.unlimitedFlying = true
    f.active_item, f.pitch_mode, f.pitch_input = f.item_id, "YawSpeedPitchSpeed", -1
    f.pitch_degrees = -100
    f:tick()
    equal(f.glider.equipment.gliderConfig.pitchAngleSpeed, 70, "inverted up vector restores the native positive rate instead of using folded pitch")
    assert(f.detail:find("inverted recovery", 1, true), "recovery is reported")
    f.pitch_degrees = -80
    f:tick()
    equal(f.glider.equipment.gliderConfig.pitchAngleSpeed, 70, "holding climb cannot immediately restart the negative rate after inversion")
    f.pitch_input = 0
    f:tick()
    equal(f.glider.equipment.gliderConfig.pitchAngleSpeed, 0, "releasing climb while upright clears the recovery latch")
    f.pitch_input = -1
    f:tick()
    equal(f.glider.equipment.gliderConfig.pitchAngleSpeed, -20, "a new climb command uses the bounded rate again")
    f.delta_seconds = 0
    f:tick()
    equal(f.glider.equipment.gliderConfig.pitchAngleSpeed, 70, "an invalid measured interval restores rather than retaining a negative rate")
    f.delta_seconds = 0/0
    f:tick()
    equal(f.glider.equipment.gliderConfig.pitchAngleSpeed, 70, "a nonfinite interval cannot authorize steering")
    f:unload()
end

do
    local f = fixture()
    f:tick()
    equal(f.writes, 0, "defaults do not modify original inputs")
    f.settings.unlimitedFlying = true
    f.settings.unlimitedGliderStamina = true
    f.settings.turnSpeed = 200
    f.settings.moveSpeed = 150
    f.settings.liftSpeed = 200
    f.settings.descendSpeed = 50
    f.active_item = f.item_id
    f:tick()
    equal(f.attributes[11].StaminaDepletionMod_Glider, 0, "original glider stamina input")
    equal(f.glider.equipment.gliderConfig.yawAngleSpeed, 110, "turn multiplier")
    equal(f.glider.equipment.gliderConfig.accelerationForward, 1.5, "move multiplier")
    equal(f.glider.equipment.gliderConfig.airResistanceLongitudinal, string.unpack("f", string.pack("f", 0.15500000119209 * 2)), "lift coefficient multiplier")
    equal(f.glider.equipment.gliderConfig.airResistanceVertical, 1.5, "descend coefficient multiplier")
    local applied_writes = f.writes
    f:tick()
    equal(f.writes, applied_writes, "does not compound or rewrite unchanged settings")

    f.settings.turnSpeed = 100
    f:tick()
    equal(f.glider.equipment.gliderConfig.yawAngleSpeed, 55, "returning a slider to 100 percent restores baseline")
    f.glider.equipment.gliderConfig.accelerationForward = 1.7
    f:tick()
    equal(f.glider.equipment.gliderConfig.accelerationForward, 1.7, "external resource update is preserved")
    f.settings = {}
    f:tick()
    equal(f.attributes[11].StaminaDepletionMod_Glider, 0.35, "stamina baseline restored")
    equal(f.glider.equipment.gliderConfig.airResistanceLongitudinal, string.unpack("f", string.pack("f", 0.155)), "lift baseline restored")
    equal(f.glider.equipment.gliderConfig.airResistanceVertical, 3, "descend baseline restored")
    equal(f.glider.equipment.gliderConfig.accelerationForward, 1.7, "external resource update retained on disable")
    f:unload()
end

do
    local f = fixture()
    f.active_item = f.item_id
    f.settings.turnSpeed = 200
    f.block_after_write = true
    f:tick()
    equal(f.glider.equipment.gliderConfig.yawAngleSpeed, 110, "confirmed write survives a later busy read")
    equal(f.writes, 1)
    assert(f.state ~= "write-failed", f.detail)
    f:tick()
    equal(f.writes, 1, "busy reads preserve the original ownership baseline")
    f.read_busy, f.block_after_write = false, false
    f:tick()
    equal(f.writes, 1, "read recovery does not multiply the already modified value")
    f.settings.turnSpeed = 300
    f:tick()
    equal(f.glider.equipment.gliderConfig.yawAngleSpeed, 165, "changed multiplier still uses the original 55 baseline")
    f.settings.turnSpeed = 100
    f.read_busy = true
    f:tick()
    equal(f.glider.equipment.gliderConfig.yawAngleSpeed, 165, "busy restore retains ownership")
    f.read_busy = false
    f:tick()
    equal(f.glider.equipment.gliderConfig.yawAngleSpeed, 55, "restore retries the preserved original baseline")
    f:unload()
end

do
    local f = fixture()
    f.players = {11}
    f.settings.unlimitedFlying = true
    f.settings.unlimitedGliderStamina = true
    f.settings.turnSpeed = 200
    f.active_item = f.item_id
    f:tick()
    equal(f.attributes[11].StaminaDepletionMod_Glider, 0, "stamina setting works before a glider action")
    equal(f.writes, 2)
    f.session = f.session + 1
    f:tick()
    equal(f.state, "write-failed", "shared item setting cannot cross sessions")
    equal(f.writes, 2, "no write is authorized against the old session")
end

do
    local f = fixture()
    local source = game.assets.get_loaded_resource("80c9eaeb-7fa0-480d-a55f-91c2c51b383e", "keen::ItemInfo", 0)
    local target = assert(runtime.overrides.resource(source, "equipment.gliderConfig.yawAngleSpeed"))
    local observed = assert(target.read())
    f.session = f.session + 1
    local current, reason = target.read()
    equal(current, nil, "generic resource target remains bound to its creation world")
    assert(reason)
    local applied = target.write(observed, 110)
    equal(applied, false, "a retained target cannot restore or change a later world's resource")
    equal(f.writes, 0, "generic target session guard runs before the writer")
    equal(f.glider.equipment.gliderConfig.yawAngleSpeed, 55)
end

do
    local f = fixture()
    f.settings.unlimitedFlying = false
    f.active_item, f.pitch_mode, f.pitch_input = f.item_id, "YawSpeedPitchSpeed", -1
    f.key_pressed = true
    f:tick()
    equal(f.glider.equipment.gliderConfig.pitchAngleSpeed, -70, "F6 toggles the local upward-gliding state on")
    assert(f.detail:find("selected Upward Gliding ON", 1, true), "F6 reports the local selection")
    equal(#f.outgoing, 0, "singleplayer does not create a remote request")
    f.key_pressed = true
    f:tick()
    equal(f.glider.equipment.gliderConfig.pitchAngleSpeed, 70, "the second F6 press restores the native rate")
    assert(f.detail:find("selected Upward Gliding OFF", 1, true), "the second press reports the new local selection")
    f:unload()
end

do
    local f = fixture()
    local peer = "76561198000000001"
    f.settings.unlimitedFlying = true
    f.remote_server = peer
    f.active_item, f.pitch_mode, f.pitch_input = f.item_id, "YawSpeedPitchSpeed", -1
    f.key_pressed = true
    f:tick()
    equal(f.glider.equipment.gliderConfig.pitchAngleSpeed, 70, "F6 off restores the local pitch rate")
    assert(f.detail:find("has not confirmed", 1, true), "local state remains distinct from server confirmation")
    local client_nonce = assert(f.outgoing[1].payload:match("^H|1|([%w_%-]+)$"))
    f.incoming = {network_message(peer, "H|1|" .. client_nonce .. "|server-session-1|1")}
    f:tick()
    local request_nonce, server_epoch, sequence, desired = f.outgoing[2].payload:match(
        "^S|1|([%w_%-]+)|([%w_%-]+)|(%d+)|([01])$")
    equal(request_nonce, client_nonce)
    equal(server_epoch, "server-session-1")
    equal(sequence, "1")
    equal(desired, "0")
    assert(f.detail:find("has not confirmed", 1, true), "transport acceptance does not report an enabled server effect")
    equal(f.writes, 0, "a client request does not write a server-side effect")

    f.incoming = {network_message(peer,
        table.concat({"R", "1", client_nonce, server_epoch, sequence, "unsupported",
            "authoritative_requester_mapping_unavailable"}, "|"))}
    f:tick()
    assert(f.detail:find("response: unsupported", 1, true), "server limitation is shown explicitly")
    assert(f.detail:find("No server Flight state was changed", 1, true), "unsupported response confirms no remote mutation")
    equal(f.writes, 0)
    f:unload()
end

do
    local f = fixture()
    f.settings.unlimitedFlying = false
    f.active_item, f.pitch_mode, f.pitch_input = f.item_id, "YawSpeedPitchSpeed", -1
    f.key_pressed = true
    f:tick()
    equal(f.glider.equipment.gliderConfig.pitchAngleSpeed, -70, "one keybind press enables this session's temporary flight state")
    equal(f.settings.unlimitedFlying, false, "the temporary toggle does not rewrite the saved setting")

    f:tick()
    equal(f.glider.equipment.gliderConfig.pitchAngleSpeed, -70, "a held key is not treated as another press by the one-shot API")
    f.active_item = 0
    f:tick()
    equal(f.glider.equipment.gliderConfig.pitchAngleSpeed, 70, "landing restores the native pitch before the session changes")

    f.session = f.session + 1
    f.active_item = f.item_id
    f:tick()
    equal(f.glider.equipment.gliderConfig.pitchAngleSpeed, 70, "a new world session clears the previous temporary toggle")
    f:unload()
end

do
    local f = fixture()
    f.settings.unlimitedFlying = true
    f.active_item, f.pitch_mode, f.pitch_input = f.item_id, "YawSpeedPitchSpeed", -1
    f.key_pressed = true
    f:tick()
    equal(f.glider.equipment.gliderConfig.pitchAngleSpeed, 70, "F6 temporarily overrides the saved enabled value")

    f.settings.unlimitedFlying = false
    f:tick()
    equal(f.glider.equipment.gliderConfig.pitchAngleSpeed, 70, "a live saved-setting change clears the temporary override")
    f.settings.unlimitedFlying = true
    f:tick()
    equal(f.glider.equipment.gliderConfig.pitchAngleSpeed, -70, "the next saved-setting change remains authoritative")
    f:unload()
end

do
    local f = fixture()
    local peer = "76561198000000001"
    f.settings.unlimitedFlying = true
    f.remote_server = peer
    f.key_pressed = true
    f:tick()
    local client_nonce = assert(f.outgoing[1].payload:match("^H|1|([%w_%-]+)$"))

    local wrong_peer = network_message("76561198000000002",
        "H|1|" .. client_nonce .. "|server-session-1|1")
    local wrong_mod = network_message(peer, "H|1|" .. client_nonce .. "|server-session-1|1")
    wrong_mod.from_mod = "another-mod"
    local malformed = network_message(peer, "H|1|malformed-token!|server-session-1|1")
    f.incoming = {wrong_peer, wrong_mod, malformed}
    f:tick()
    equal(#f.outgoing, 1, "wrong-peer, wrong-envelope, and malformed replies do not establish a session")

    f.incoming = {network_message(peer, "H|1|" .. client_nonce .. "|server-session-1|1")}
    f:tick()
    local request_nonce, server_epoch, sequence = f.outgoing[2].payload:match(
        "^S|1|([%w_%-]+)|([%w_%-]+)|(%d+)|[01]$")
    equal(request_nonce, client_nonce)
    equal(server_epoch, "server-session-1")
    equal(sequence, "1")

    local wrong_peer_result = network_message("76561198000000002",
        table.concat({"R", "1", request_nonce, server_epoch, sequence, "unsupported",
            "authoritative_requester_mapping_unavailable"}, "|"))
    local wrong_sequence = network_message(peer,
        table.concat({"R", "1", request_nonce, server_epoch, "2", "unsupported",
            "authoritative_requester_mapping_unavailable"}, "|"))
    f.incoming = {wrong_peer_result, wrong_sequence, network_message(peer, "R|1|broken")}
    f:tick()
    assert(f.detail:find("has not confirmed", 1, true), "unmatched replies cannot claim a server effect")
    equal(f.writes, 0, "unmatched replies do not change local or shared game values")

    f.delta_seconds = 5
    f:tick()
    assert(f.detail:find("server Flight state is unknown", 1, true), "timeout reports the server state as unknown")
    assert(f.state ~= "write-confirmed", "a missing reply cannot report a confirmed write")
    equal(f.writes, 0, "timeout does not convert transport delivery into a server-side effect")
    f:unload()
end

do
    local f = fixture()
    local peer = "76561198000000001"
    f.server = true
    f.connected_peers = {peer}

    local wrong_source = network_message(peer, "H|1|client-epoch-1")
    wrong_source.from_mod = "another-mod"
    local wrong_target = network_message(peer, "H|1|client-epoch-2")
    wrong_target.to_mod = "another-mod"
    local unreliable = network_message(peer, "H|1|client-epoch-3")
    unreliable.reliable = false
    local oversized = network_message(peer, string.rep("x", 257))
    f.incoming = {wrong_source, wrong_target, unreliable, oversized,
        network_message(peer, "H|1|malformed-token!")}
    f:tick()
    equal(#f.outgoing, 0, "connected peers still need the exact reliable mod envelope and a valid payload")
    equal(f.writes, 0, "malformed or misrouted requests cannot change flight values")

    f.incoming = {network_message(peer, "H|1|client-epoch-1")}
    f:tick()
    local client_nonce, server_epoch = f.outgoing[1].payload:match(
        "^H|1|([%w_%-]+)|([%w_%-]+)|%d+$")
    equal(client_nonce, "client-epoch-1")

    f.incoming = {network_message(peer,
        table.concat({"S", "1", client_nonce, server_epoch, "2", "1"}, "|"))}
    f:tick()
    assert(f.outgoing[2].payload:find("|rejected|out_of_order_request$"), "skipped request sequences are rejected")

    f.incoming = {network_message(peer,
        table.concat({"S", "1", client_nonce, server_epoch, "1", "1"}, "|"))}
    f:tick()
    assert(f.outgoing[3].payload:find("|unsupported|authoritative_requester_mapping_unavailable$"),
        "the next in-order request reaches the explicit unsupported result")
    f.incoming = {network_message(peer,
        table.concat({"S", "1", client_nonce, server_epoch, "1", "1"}, "|"))}
    f:tick()
    assert(f.outgoing[4].payload:find("|rejected|stale_request$"), "unsupported requests still consume freshness")
    equal(f.writes, 0, "no authenticated client request changes server flight state without requester mapping")
    equal(f.glider.equipment.gliderConfig.pitchAngleSpeed, 70, "unsupported server requests preserve shared ItemInfo")
    equal(f.attributes[11].StaminaDepletionMod_Glider, 0.35, "unsupported server requests preserve authoritative stamina")
    f:unload()
end

do
    local f = fixture()
    local peer = "76561198000000001"
    f.server = true
    f.connected_peers = {peer}
    f.incoming = {network_message(peer, "H|1|client-epoch-1")}
    f:tick()
    local client_nonce, server_epoch, next_sequence = f.outgoing[1].payload:match(
        "^H|1|([%w_%-]+)|([%w_%-]+)|(%d+)$")
    equal(client_nonce, "client-epoch-1")
    equal(next_sequence, "1")

    f.incoming = {network_message(peer,
        table.concat({"S", "1", client_nonce, "stale-server-epoch", "1", "1"}, "|"))}
    f:tick()
    assert(f.outgoing[2].payload:find("|rejected|stale_server_session$"), "old server session is rejected")

    f.incoming = {network_message(peer,
        table.concat({"S", "1", client_nonce, server_epoch, "1", "1"}, "|"))}
    f:tick()
    assert(f.outgoing[3].payload:find("|unsupported|authoritative_requester_mapping_unavailable$"),
        "authenticated request receives explicit unsupported response")

    f.incoming = {network_message(peer,
        table.concat({"S", "1", client_nonce, server_epoch, "1", "1"}, "|"))}
    f:tick()
    assert(f.outgoing[4].payload:find("|rejected|stale_request$"), "duplicate request sequence is rejected")
    equal(f.writes, 0, "server protocol never changes flight state without requester mapping")
    equal(f.attributes[11].StaminaDepletionMod_Glider, 0.35, "remote requests preserve server stamina state")
    equal(f.glider.equipment.gliderConfig.pitchAngleSpeed, 70, "remote requests preserve shared glider pitch")
    f:unload()
end

do
    local f = fixture()
    local peer = "76561198000000001"
    f.server = true
    f.connected_peers = {}
    f.incoming = {network_message(peer, "H|1|unauthorized-client")}
    f:tick()
    equal(#f.outgoing, 0, "unauthenticated peer receives no Flight protocol response")
    equal(f.writes, 0, "unauthenticated peer cannot change flight state")
    f:unload()
end
