cmake_minimum_required(VERSION 3.20)

project(anpod_audio_capture LANGUAGES CXX)

# The managed side is x64-only (plan §1.2); a 32-bit DLL would load-fail at
# runtime with a confusing error, so fail loudly at configure time instead.
if(NOT CMAKE_SIZEOF_VOID_P EQUAL 8)
    message(FATAL_ERROR "AnPod native capture must be built for x64 (Win64).")
endif()

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

add_library(audio_capture SHARED
    audio_capture.cpp
    audio_capture.h
)

# Exports are declared with __declspec(dllexport) in audio_capture.h, so no
# .def file is needed.
set_target_properties(audio_capture PROPERTIES
    OUTPUT_NAME "audio_capture"
    PREFIX ""
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/bin"
    RUNTIME_OUTPUT_DIRECTORY_DEBUG "${CMAKE_BINARY_DIR}/bin"
    RUNTIME_OUTPUT_DIRECTORY_RELEASE "${CMAKE_BINARY_DIR}/bin"
)

if(MSVC)
    set_property(TARGET audio_capture PROPERTY
        MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>"
    )
endif()

# ws2_32 / winmm / ole32 / avrt are also pulled in via #pragma comment for MSVC, but
# listing them keeps MinGW and any other toolchain working.
target_link_libraries(audio_capture PRIVATE ws2_32 winmm ole32 avrt uuid)

# WASAPI stream auto-conversion flags require Windows 7 or later. Keep the
# compile-time target aligned with that runtime floor.
target_compile_definitions(audio_capture PRIVATE _WIN32_WINNT=0x0601)

if(MSVC)
    target_compile_options(audio_capture PRIVATE /W4 /permissive-)
    target_compile_definitions(audio_capture PRIVATE _CRT_SECURE_NO_WARNINGS)
else()
    target_compile_options(audio_capture PRIVATE -Wall -Wextra)
endif()
