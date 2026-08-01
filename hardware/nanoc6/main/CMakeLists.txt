set(CASE_IDS
    0000 0025 0050 0075 0100 0125 0150 0175 0200 0225
    0250 0275 0300 0325 0350 0375 0400 0425 0450 0475
    0500 0525 0550 0575 0600 0625 0650 0675 0700 0725
    0750 0775 0800 0825 0850 0875 0900 0925 0950 0975
)

set(CASE_SOURCES)
foreach(CASE_ID IN LISTS CASE_IDS)
    set(CASE_SOURCE
        "${CMAKE_CURRENT_LIST_DIR}/../../../corpus/edge_cases/cases/case_${CASE_ID}.c")
    list(APPEND CASE_SOURCES "${CASE_SOURCE}")
    if(NOT CMAKE_SCRIPT_MODE_FILE)
        set_source_files_properties(
            "${CASE_SOURCE}"
            PROPERTIES COMPILE_DEFINITIONS "remu_case=remu_case_${CASE_ID}"
        )
    endif()
endforeach()

idf_component_register(
    SRCS "app_main.c" ${CASE_SOURCES}
    INCLUDE_DIRS "."
)

target_compile_options(
    ${COMPONENT_LIB}
    PRIVATE
        -O2
        -fno-builtin
        -Wno-error=parentheses
        -Wno-error=unused-function
)
