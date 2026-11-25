# Swift Note Rust static library integration (swiftnote-rs)
find_program(SWIFTNOTE_CARGO_EXECUTABLE cargo REQUIRED)

set(SWIFTNOTE_RUST_MANIFEST "${PROJECT_SOURCE_DIR}/rust/swiftnote-rs/Cargo.toml")
set(SWIFTNOTE_RUST_TARGET_DIR "${CMAKE_BINARY_DIR}/rust-target")
set(SWIFTNOTE_RUST_INCLUDE_DIR "${PROJECT_SOURCE_DIR}/rust/swiftnote-rs/include")

if (CMAKE_BUILD_TYPE STREQUAL "Debug")
    set(SWIFTNOTE_RUST_PROFILE "debug")
    set(SWIFTNOTE_CARGO_PROFILE_ARGS "")
else ()
    set(SWIFTNOTE_RUST_PROFILE "release")
    set(SWIFTNOTE_CARGO_PROFILE_ARGS "--release")
endif ()

if (MSVC)
    set(SWIFTNOTE_RUST_LIB_NAME "swiftnote_rs.lib")
else ()
    set(SWIFTNOTE_RUST_LIB_NAME "libswiftnote_rs.a")
endif ()

set(SWIFTNOTE_RUST_LIB_PATH "${SWIFTNOTE_RUST_TARGET_DIR}/${SWIFTNOTE_RUST_PROFILE}/${SWIFTNOTE_RUST_LIB_NAME}")

file(GLOB_RECURSE SWIFTNOTE_RUST_SOURCES
    "${PROJECT_SOURCE_DIR}/rust/swiftnote-rs/src/*.rs"
    "${PROJECT_SOURCE_DIR}/rust/swiftnote-rs/Cargo.toml"
)

add_custom_command(
    OUTPUT "${SWIFTNOTE_RUST_LIB_PATH}"
    COMMAND "${SWIFTNOTE_CARGO_EXECUTABLE}" build
        --manifest-path "${SWIFTNOTE_RUST_MANIFEST}"
        --target-dir "${SWIFTNOTE_RUST_TARGET_DIR}"
        ${SWIFTNOTE_CARGO_PROFILE_ARGS}
    DEPENDS ${SWIFTNOTE_RUST_SOURCES}
    WORKING_DIRECTORY "${PROJECT_SOURCE_DIR}/rust/swiftnote-rs"
    COMMENT "Building swiftnote-rs (${SWIFTNOTE_RUST_PROFILE})"
    VERBATIM
)

add_custom_target(swiftnote_rs_build DEPENDS "${SWIFTNOTE_RUST_LIB_PATH}")

add_library(swiftnote_rs STATIC IMPORTED GLOBAL)
add_dependencies(swiftnote_rs swiftnote_rs_build)
set_target_properties(swiftnote_rs PROPERTIES
    IMPORTED_LOCATION "${SWIFTNOTE_RUST_LIB_PATH}"
    INTERFACE_INCLUDE_DIRECTORIES "${SWIFTNOTE_RUST_INCLUDE_DIR}"
)

if (WIN32)
    set_property(TARGET swiftnote_rs APPEND PROPERTY INTERFACE_LINK_LIBRARIES
        ws2_32 userenv bcrypt ntdll advapi32 synchronization ole32 shell32
    )
elseif (UNIX)
    set_property(TARGET swiftnote_rs APPEND PROPERTY INTERFACE_LINK_LIBRARIES
        pthread dl m fontconfig
    )
endif ()

add_library(swiftnote::rs ALIAS swiftnote_rs)
