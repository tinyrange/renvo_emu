/* SPDX-License-Identifier: Apache-2.0 */
#ifndef REMU_C6_CRYPTO_H
#define REMU_C6_CRYPTO_H

#include <stdint.h>

void c6_pbkdf2_sha1(const uint8_t *passphrase, uint32_t passphrase_length,
                    const uint8_t *ssid, uint32_t ssid_length,
                    uint8_t output[32]);
void c6_wpa_prf(const uint8_t pmk[32], const uint8_t address_a[6],
                const uint8_t address_b[6], const uint8_t nonce_a[32],
                const uint8_t nonce_b[32], uint8_t ptk[64]);
void c6_hmac_sha1(const uint8_t *key, uint32_t key_length,
                  const uint8_t *message, uint32_t message_length,
                  uint8_t output[20]);

#endif
