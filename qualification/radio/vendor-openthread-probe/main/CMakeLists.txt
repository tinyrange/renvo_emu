# SPDX-License-Identifier: Apache-2.0
idf_component_register(SRCS "vendor_openthread_probe.c"
                       INCLUDE_DIRS "."
                       REQUIRES openthread nvs_flash esp_event vfs)
