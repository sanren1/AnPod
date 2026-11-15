cmake_minimum_required(VERSION 3.5.0)
project(anpod_core)

add_library(anpod_core SHARED napi_entry.cpp crypto.cpp pairing_protocol.cpp
    anpu_codec.cpp anhb_codec.cpp p5c_control.cpp anpd_ingress.cpp self_test.cpp)
target_link_libraries(anpod_core PUBLIC
    libace_napi.z.so libohcrypto.so libasset_ndk.z.so libohaudio.so)
