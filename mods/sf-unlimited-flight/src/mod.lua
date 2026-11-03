local overrides = runtime.overrides.new()
local pitch_overrides = runtime.overrides.new()
local active_session
local last_state, last_detail
local gliders = {}
local personal_on = false

local item_id_path = "currentAction.triggerContext.usedItem.itemId"
local glider_fields = {
    {setting="turnSpeed", path="equipment.gliderConfig.yawAngleSpeed"},
    {setting="moveSpeed", path="equipment.gliderConfig.accelerationForward"},
    {setting="liftSpeed", path="equipment.gliderConfig.airResistanceLongitudinal"},
    {setting="descendSpeed", path="equipment.gliderConfig.airResistanceVertical"},
}

local function report(state, detail)
    if last_state == state and last_detail == detail then return end
    last_state, last_detail = state, detail
    runtime.report_effect(state, detail)
end

local function finite(value)
    return type(value) == "number" and value == value and math.abs(value) < math.huge
end

local function setting_enabled(key)
    if key == "unlimitedGliderStamina" then return false end
    if key == "unlimitedFlying" then
        return shroudforge.settings.get(key, false) == true
    end
    local value = shroudforge.settings.get(key, 100)
    return finite(value) and value ~= 100
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

-- The native controller owns personal selection and sender resets. Lua owns
-- only the declared SF keybind and notices. Never write shared pitch for F6.
local function reset_personal()
    personal_on = false
    if runtime.flight then runtime.flight.reset_local() end
end

local function update_personal()
    pitch_overrides:restore_all()
    if pitch_overrides.fault or pitch_overrides:has_ownership() then
        reset_personal()
        return false, "Waiting for the previous shared pitch override to restore"
    end
    if not runtime.is_client or runtime.is_server then return false end
    if not runtime.flight then return false, "Native Flight binding is unavailable" end
    local status = runtime.flight.status()
    personal_on = status.requested_on == true
    if not setting_enabled("unlimitedFlying") then
        reset_personal()
        return false
    end
    if shroudforge.input.was_keybind_pressed("toggleFlight") then
        local requested = not personal_on
        local accepted, reason = runtime.flight.set_local_enabled(requested)
        status = runtime.flight.status()
        personal_on = status.requested_on == true
        if accepted and personal_on == requested then
            shroudforge.notifications.publish({id="flight-toggle", title="Unlimited Glider Flight",
                message=personal_on and "Unlimited Glider Flight On" or "Unlimited Glider Flight Off", level="info"})
        else
            return false, tostring(reason or "Native Flight is waiting for qualified input")
        end
    end
    return personal_on
end

local function update(_delta_seconds)
    local session = runtime.world.session_id()
    if session == 0 then
        reset_personal()
        report("waiting", "Waiting for an authoritative world") return
    end
    if active_session and session ~= active_session then
        reset_personal()
        gliders = {}
    end
    active_session = session
    overrides:begin_update()
    pitch_overrides:begin_update()

    overrides:restore_disabled(setting_enabled)
    pitch_overrides:restore_disabled(setting_enabled)
    if overrides.fault then reset_personal() report("write-failed", overrides.fault) return end
    if pitch_overrides.fault then reset_personal() report("write-failed", pitch_overrides.fault) return end
    local native_on, native_reason = update_personal()
    if native_reason then report("waiting", native_reason) end
    if not has_effect() then
        report(overrides:has_ownership() and "waiting" or "no-change",
            overrides:summary("Unlimited Flying and all speed multipliers are at their defaults"))
        return
    end
    local shared_speed = false
    for _, field in ipairs(glider_fields) do
        if setting_enabled(field.setting) then shared_speed = true end
    end
    if not shared_speed then
        report("waiting", native_reason or (native_on and "Personal Flight On intent; server effect is not acknowledged" or "Personal Flight Off; stamina unchanged"))
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
        pitch_overrides:restore_all()
        report("no-target", "No live player entity is available") return
    end

    for _, entity in ipairs(players) do
        local item_id, item_reason = active_item(entity)
        if item_id then
            local resource, resource_reason = current_glider(item_id)
            if resource then
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
            overrides:waiting("Glider settings: " .. tostring(item_reason))
        end
    end

    pitch_overrides:restore_unused()

    local state = (overrides.fault or pitch_overrides.fault) and "write-failed" or
        (overrides.waiting_count + pitch_overrides.waiting_count > 0 and "waiting" or
        (overrides.writes + pitch_overrides.writes > 0 and "write-confirmed" or "no-change"))
    report(state, pitch_overrides:summary(overrides:summary("Shared glider speed settings")))
end

return {
    update_interval_ms = 100,
    on_load = function()
        personal_on = false
        if runtime.flight and runtime.is_client and not runtime.is_server then runtime.flight.set_local_enabled(false) end
        if type(game.assets.prepare_loaded_index) == "function" then
            game.assets.prepare_loaded_index("keen::ItemInfo", "itemId.value")
        end
        report("waiting", "SF Unlimited Flight loaded. Waiting for typed engine access")
    end,
    on_update = update,
    on_unload = reset_personal,
}
