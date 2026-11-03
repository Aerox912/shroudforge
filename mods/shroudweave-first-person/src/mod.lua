local DEFAULT_CAMERA_DISTANCE = -0.15

local active = false
local camera_distance = DEFAULT_CAMERA_DISTANCE
local last_error
local reported_error

local function finite(value)
    return type(value) == "number" and value == value and math.abs(value) < math.huge
end

local function read_camera_distance()
    if type(shroudforge) ~= "table" or type(shroudforge.settings) ~= "table"
        or type(shroudforge.settings.get) ~= "function" then
        return camera_distance
    end
    local ok, value = pcall(shroudforge.settings.get, "cameraDistance", DEFAULT_CAMERA_DISTANCE)
    if ok and finite(value) and value >= -0.5 and value <= 0.5 then
        return value
    end
    return camera_distance
end

local function set_mode(enabled, distance)
    if type(runtime.first_person) ~= "table"
        or type(runtime.first_person.set_mode) ~= "function" then
        last_error = "The native first-person runtime API is unavailable."
        return false
    end

    local called, changed, detail = pcall(runtime.first_person.set_mode, enabled, distance)
    if not called then
        last_error = tostring(changed)
        return false
    end
    if changed ~= true then
        last_error = detail or "The first-person mode change was rejected."
        return false
    end

    active = enabled
    camera_distance = distance
    last_error = nil
    return true
end

local function report_error()
    if not last_error then
        reported_error = nil
        return
    end
    if last_error ~= reported_error and type(runtime.report_effect) == "function" then
        pcall(runtime.report_effect, "waiting", last_error)
        reported_error = last_error
    end
end

local function update_keybind()
    if type(shroudforge) ~= "table" or type(shroudforge.input) ~= "table"
        or type(shroudforge.input.was_keybind_pressed) ~= "function" then
        return
    end
    local ok, pressed = pcall(shroudforge.input.was_keybind_pressed, "toggleFirstPerson")
    if ok and pressed == true then
        set_mode(not active, camera_distance)
    end
end

return {
    update_interval_ms = 100,
    on_load = function()
        active = false
        camera_distance = read_camera_distance()
        set_mode(false, camera_distance)
        -- Establish the hotkey revision before the first update callback so
        -- an early press cannot be mistaken for the initial baseline.
        update_keybind()
        report_error()
    end,
    on_update = function()
        local configured_distance = read_camera_distance()
        if configured_distance ~= camera_distance then
            set_mode(active, configured_distance)
        end
        update_keybind()
        report_error()
    end,
    on_unload = function()
        set_mode(false, camera_distance)
        report_error()
    end,
}
