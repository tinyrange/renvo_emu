# SPDX-License-Identifier: Apache-2.0
idf_component_register(SRCS "vendor_zigbee_probe.c"
                       INCLUDE_DIRS "."
                       REQUIRES nvs_flash)
