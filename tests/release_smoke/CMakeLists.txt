# SPDX-License-Identifier: Apache-2.0
cmake_minimum_required(VERSION 3.21)
project(wlc_archived_compiler_smoke LANGUAGES C)
include(CTest)
find_package(Wirelink 0.13.0 EXACT CONFIG REQUIRED)
wirelink_wlc_generate(TARGET control_protocol
  SCHEMA "${CMAKE_CURRENT_SOURCE_DIR}/../fixtures/release_smoke.wl"
  PROFILE "${CMAKE_CURRENT_SOURCE_DIR}/../fixtures/release_smoke.bind.wl")
add_executable(release_roundtrip main.c)
target_link_libraries(release_roundtrip PRIVATE control_protocol Wirelink::packet_loopback)
if(MSVC)
  target_compile_options(release_roundtrip PRIVATE /W4 /WX)
else()
  target_compile_options(release_roundtrip PRIVATE -Wall -Wextra -Wpedantic -Werror)
endif()
add_test(NAME archived_compiler_rpc_roundtrip COMMAND release_roundtrip)
set_tests_properties(archived_compiler_rpc_roundtrip PROPERTIES TIMEOUT 10)
