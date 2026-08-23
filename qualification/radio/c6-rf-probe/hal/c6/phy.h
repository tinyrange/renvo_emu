/* SPDX-License-Identifier: Apache-2.0 */
#ifndef REMU_C6_PHY_H
#define REMU_C6_PHY_H

enum c6_phy_result {
    C6_PHY_OK = 0,
    C6_PHY_RESET_NOT_READY = -1,
};

int c6_phy_init(void);
int c6_phy_reset_radio(void);

#endif
