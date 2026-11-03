# Minimal static target wrapper for the exact hash-pinned MinHook inputs.
# The wrapper is copied to a temporary dependency staging directory at build time.
add_library(minhook STATIC
    src/buffer.c
    src/hook.c
    src/trampoline.c
    src/hde/hde64.c)
target_include_directories(minhook PUBLIC "${CMAKE_CURRENT_SOURCE_DIR}/include")
target_include_directories(minhook PRIVATE
    "${CMAKE_CURRENT_SOURCE_DIR}/src"
    "${CMAKE_CURRENT_SOURCE_DIR}/src/hde")
set_target_properties(minhook PROPERTIES
    C_STANDARD 99
    C_STANDARD_REQUIRED ON
    C_EXTENSIONS OFF)
