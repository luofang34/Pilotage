cmake_minimum_required(VERSION 3.16)
project(pilotage_gz_bridge CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

if(NOT CMAKE_BUILD_TYPE)
  set(CMAKE_BUILD_TYPE Release)
endif()

# gz-transport / gz-msgs are discovered via pkg-config: their .pc files pull in
# protobuf, abseil, gz-math, and gz-utils transitively, so no separate
# find_package cascade is needed.
find_package(PkgConfig REQUIRED)
pkg_check_modules(GZ_TRANSPORT REQUIRED IMPORTED_TARGET gz-transport13)
pkg_check_modules(GZ_MSGS REQUIRED IMPORTED_TARGET gz-msgs10)

find_program(PROTOC_EXECUTABLE protoc REQUIRED)

# Generate C++ bindings for the private host<->sidecar bridge schema. The
# schema lives at the workspace root; regenerate whenever it changes.
set(SCHEMA_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/../../../schemas")
set(BRIDGE_PROTO "${SCHEMA_ROOT}/pilotage/bridge/v1/bridge.proto")
set(GENERATED_DIR "${CMAKE_CURRENT_BINARY_DIR}/generated")
set(BRIDGE_PB_CC "${GENERATED_DIR}/pilotage/bridge/v1/bridge.pb.cc")
set(BRIDGE_PB_H "${GENERATED_DIR}/pilotage/bridge/v1/bridge.pb.h")

file(MAKE_DIRECTORY "${GENERATED_DIR}")

add_custom_command(
  OUTPUT "${BRIDGE_PB_CC}" "${BRIDGE_PB_H}"
  COMMAND "${PROTOC_EXECUTABLE}"
          --proto_path "${SCHEMA_ROOT}"
          --cpp_out "${GENERATED_DIR}"
          "${BRIDGE_PROTO}"
  DEPENDS "${BRIDGE_PROTO}"
  COMMENT "Generating C++ bindings for pilotage.bridge.v1"
  VERBATIM)

add_executable(pilotage-gz-bridge
  src/main.cpp
  src/framing.cpp
  src/bridge_node.cpp
  "${BRIDGE_PB_CC}")

target_include_directories(pilotage-gz-bridge PRIVATE
  "${CMAKE_CURRENT_SOURCE_DIR}/src"
  "${GENERATED_DIR}")

target_link_libraries(pilotage-gz-bridge PRIVATE
  PkgConfig::GZ_TRANSPORT
  PkgConfig::GZ_MSGS)

target_compile_options(pilotage-gz-bridge PRIVATE -Wall -Wextra)
