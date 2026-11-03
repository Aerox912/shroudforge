# Private native Flight + single shared Vein/Flight provider.
# No host/bootstrap registration lives here. RB11 owns demand/lifecycle calls.
set(vf "${CMAKE_CURRENT_SOURCE_DIR}/../vein_flight")
add_library(sf-vein-flight-minhook STATIC
    "${vf}/third_party/minhook/src/buffer.c"
    "${vf}/third_party/minhook/src/hook.c"
    "${vf}/third_party/minhook/src/trampoline.c"
    "${vf}/third_party/minhook/src/hde/hde64.c")
target_include_directories(sf-vein-flight-minhook PUBLIC "${vf}/third_party/minhook/include")
set_property(TARGET sf-vein-flight-minhook PROPERTY MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")
set_property(TARGET sf-vein-flight-minhook PROPERTY C_STANDARD 99)

target_sources(kfc-runtime PRIVATE
    "${vf}/provider_adapter.cpp" "${vf}/a6_consumer.cpp"
    "${vf}/consumer_support/src/controls.cpp"
    "${vf}/consumer_support/src/settings_file_win32.cpp"
    "${vf}/consumer_support/src/coordinator.cpp"
    "${vf}/shared/src/profile.cpp" "${vf}/shared/src/original_client_guard.cpp"
    "${vf}/shared/src/flight_transport.cpp" "${vf}/shared/src/native_reset.cpp"
    "${vf}/shared/src/native_input_session.cpp"
    client_controller.cpp client_publisher.cpp client_scalar_bridge.cpp
    donor/src/flight_client_frame.cpp donor/src/flight_glider_gate.cpp
    donor/src/flight_layout.cpp donor/src/flight_scalar_sdk.cpp
    donor/src/local_subject_context.cpp)
target_include_directories(kfc-runtime PRIVATE
    "${vf}/consumer_support/include" "${vf}/shared/include"
    "${CMAKE_CURRENT_SOURCE_DIR}/donor/include")
target_link_libraries(kfc-runtime PRIVATE sf-vein-flight-minhook user32 psapi bcrypt)
# Explicit reviewed compiler profile. Refuse accidental toolchain substitution.
if(CMAKE_CXX_COMPILER_VERSION VERSION_EQUAL "19.44.35228.0")
    target_compile_definitions(kfc-runtime PRIVATE SHROUDWEAVE_FLIGHT_MSC_FULL_VER=194435228)
elseif(CMAKE_CXX_COMPILER_VERSION VERSION_EQUAL "19.44.35229.0")
    target_compile_definitions(kfc-runtime PRIVATE SHROUDWEAVE_FLIGHT_MSC_FULL_VER=194435229)
else()
    message(FATAL_ERROR "Flight requires its reviewed MSVC194435228/194435229 compiler profile")
endif()

add_executable(kfc-runtime-test-flight-owner EXCLUDE_FROM_ALL ../../tests/flight_owner_test.cpp)
target_include_directories(kfc-runtime-test-flight-owner PRIVATE "${CMAKE_CURRENT_SOURCE_DIR}/..")
target_compile_features(kfc-runtime-test-flight-owner PRIVATE cxx_std_20)
add_executable(kfc-runtime-test-flight-publisher EXCLUDE_FROM_ALL
    ../../tests/flight_publisher_test.cpp client_publisher.cpp)
add_executable(kfc-runtime-test-flight-controller EXCLUDE_FROM_ALL
    ../../tests/flight_controller_test.cpp client_controller.cpp client_publisher.cpp)
add_executable(kfc-runtime-test-flight-provider-isolation EXCLUDE_FROM_ALL
    ../../tests/flight_provider_isolation_test.cpp "${vf}/a6_consumer.cpp"
    "${vf}/consumer_support/src/controls.cpp" "${vf}/consumer_support/src/settings_file_win32.cpp"
    "${vf}/consumer_support/src/coordinator.cpp" "${vf}/shared/src/profile.cpp"
    "${vf}/shared/src/original_client_guard.cpp" "${vf}/shared/src/flight_transport.cpp"
    "${vf}/shared/src/native_reset.cpp" "${vf}/shared/src/native_input_session.cpp")
target_link_libraries(kfc-runtime-test-flight-provider-isolation PRIVATE sf-vein-flight-minhook user32 bcrypt)
foreach(flight_test kfc-runtime-test-flight-publisher kfc-runtime-test-flight-controller kfc-runtime-test-flight-provider-isolation)
    target_include_directories(${flight_test} PRIVATE "${CMAKE_CURRENT_SOURCE_DIR}/.."
        "${vf}/shared/include" "${vf}/consumer_support/include")
    target_compile_features(${flight_test} PRIVATE cxx_std_20)
    target_compile_definitions(${flight_test} PRIVATE KFC_RUNTIME_BUILD WIN32_LEAN_AND_MEAN NOMINMAX)
endforeach()
foreach(flight_test kfc-runtime-test-flight-owner kfc-runtime-test-flight-publisher kfc-runtime-test-flight-controller kfc-runtime-test-flight-provider-isolation)
    set_property(TARGET ${flight_test} PROPERTY MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")
    target_compile_options(${flight_test} PRIVATE /UNDEBUG)
endforeach()
