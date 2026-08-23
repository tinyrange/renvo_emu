set(coexistence_requirements bt esp_wifi esp_event esp_netif)
if(IDF_TARGET STREQUAL "esp32c6")
    list(APPEND coexistence_requirements ieee802154)
endif()

idf_component_register(
    SRCS "vendor_coex_probe.c"
    PRIV_REQUIRES ${coexistence_requirements}
)
