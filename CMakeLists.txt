cmake_minimum_required(VERSION 3.21)

project(CodexUsageBar VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

add_executable(CodexUsageBar WIN32
    src/main.cpp
    src/AppBarWindow.cpp
    src/CodexUsageFetcher.cpp
    src/JsonLite.cpp
)

target_include_directories(CodexUsageBar PRIVATE src)

target_compile_definitions(CodexUsageBar PRIVATE
    UNICODE
    _UNICODE
    NOMINMAX
    WIN32_LEAN_AND_MEAN
)

target_compile_options(CodexUsageBar PRIVATE /utf-8)

target_link_libraries(CodexUsageBar PRIVATE
    advapi32
    d2d1
    dwrite
    gdi32
    ole32
    shell32
    shlwapi
    user32
    winhttp
)
