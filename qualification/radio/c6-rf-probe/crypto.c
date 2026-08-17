/* SPDX-License-Identifier: Apache-2.0 */
#include "crypto.h"

/* GCC may lower fixed-size zero initialization to this freestanding helper. */
void *memset(void *destination, int value, uint32_t length)
    __attribute__((used, optimize("O0")));

void *memset(void *destination, int value, uint32_t length)
{
    volatile uint8_t *bytes = (volatile uint8_t *)destination;
    for (uint32_t index = 0; index < length; ++index) bytes[index] = (uint8_t)value;
    return destination;
}

struct sha1 {
    uint32_t state[5];
    uint32_t length;
    uint32_t used;
    uint8_t block[64];
};

static uint32_t rotate(uint32_t value, uint32_t count)
{
    return (value << count) | (value >> (32u - count));
}

static void sha1_transform(struct sha1 *context, const uint8_t block[64])
{
    uint32_t words[80];
    for (uint32_t index = 0; index < 16u; ++index) {
        words[index] = (uint32_t)block[index * 4u] << 24 |
                       (uint32_t)block[index * 4u + 1u] << 16 |
                       (uint32_t)block[index * 4u + 2u] << 8 |
                       block[index * 4u + 3u];
    }
    for (uint32_t index = 16u; index < 80u; ++index) {
        words[index] = rotate(words[index - 3u] ^ words[index - 8u] ^
                              words[index - 14u] ^ words[index - 16u], 1u);
    }
    uint32_t a = context->state[0];
    uint32_t b = context->state[1];
    uint32_t c = context->state[2];
    uint32_t d = context->state[3];
    uint32_t e = context->state[4];
    for (uint32_t index = 0; index < 80u; ++index) {
        uint32_t function;
        uint32_t constant;
        if (index < 20u) {
            function = (b & c) | (~b & d);
            constant = 0x5a827999u;
        } else if (index < 40u) {
            function = b ^ c ^ d;
            constant = 0x6ed9eba1u;
        } else if (index < 60u) {
            function = (b & c) | (b & d) | (c & d);
            constant = 0x8f1bbcdcu;
        } else {
            function = b ^ c ^ d;
            constant = 0xca62c1d6u;
        }
        uint32_t temporary = rotate(a, 5u) + function + e + constant + words[index];
        e = d;
        d = c;
        c = rotate(b, 30u);
        b = a;
        a = temporary;
    }
    context->state[0] += a;
    context->state[1] += b;
    context->state[2] += c;
    context->state[3] += d;
    context->state[4] += e;
}

static void sha1_init(struct sha1 *context)
{
    context->state[0] = 0x67452301u;
    context->state[1] = 0xefcdab89u;
    context->state[2] = 0x98badcfeu;
    context->state[3] = 0x10325476u;
    context->state[4] = 0xc3d2e1f0u;
    context->length = 0;
    context->used = 0;
}

static void sha1_update(struct sha1 *context, const uint8_t *data, uint32_t length)
{
    context->length += length;
    while (length != 0) {
        uint32_t available = 64u - context->used;
        uint32_t take = length < available ? length : available;
        for (uint32_t index = 0; index < take; ++index) {
            context->block[context->used + index] = data[index];
        }
        context->used += take;
        data += take;
        length -= take;
        if (context->used == 64u) {
            sha1_transform(context, context->block);
            context->used = 0;
        }
    }
}

static void sha1_final(struct sha1 *context, uint8_t output[20])
{
    uint32_t original_length = context->length;
    uint8_t marker = 0x80;
    sha1_update(context, &marker, 1);
    uint8_t zero = 0;
    while (context->used != 56u) sha1_update(context, &zero, 1);
    uint32_t high = original_length >> 29;
    uint32_t low = original_length << 3;
    uint8_t length[8] = {
        (uint8_t)(high >> 24), (uint8_t)(high >> 16),
        (uint8_t)(high >> 8), (uint8_t)high,
        (uint8_t)(low >> 24), (uint8_t)(low >> 16),
        (uint8_t)(low >> 8), (uint8_t)low,
    };
    sha1_update(context, length, sizeof(length));
    for (uint32_t index = 0; index < 5u; ++index) {
        output[index * 4u] = (uint8_t)(context->state[index] >> 24);
        output[index * 4u + 1u] = (uint8_t)(context->state[index] >> 16);
        output[index * 4u + 2u] = (uint8_t)(context->state[index] >> 8);
        output[index * 4u + 3u] = (uint8_t)context->state[index];
    }
}

void c6_hmac_sha1(const uint8_t *key, uint32_t key_length,
                  const uint8_t *message, uint32_t message_length,
                  uint8_t output[20])
{
    uint8_t key_block[64] = {0};
    if (key_length > 64u) {
        struct sha1 hash;
        sha1_init(&hash);
        sha1_update(&hash, key, key_length);
        sha1_final(&hash, key_block);
    } else {
        for (uint32_t index = 0; index < key_length; ++index) key_block[index] = key[index];
    }
    uint8_t inner_pad[64];
    uint8_t outer_pad[64];
    for (uint32_t index = 0; index < 64u; ++index) {
        inner_pad[index] = key_block[index] ^ 0x36u;
        outer_pad[index] = key_block[index] ^ 0x5cu;
    }
    uint8_t inner[20];
    struct sha1 hash;
    sha1_init(&hash);
    sha1_update(&hash, inner_pad, sizeof(inner_pad));
    sha1_update(&hash, message, message_length);
    sha1_final(&hash, inner);
    sha1_init(&hash);
    sha1_update(&hash, outer_pad, sizeof(outer_pad));
    sha1_update(&hash, inner, sizeof(inner));
    sha1_final(&hash, output);
}

void c6_pbkdf2_sha1(const uint8_t *passphrase, uint32_t passphrase_length,
                    const uint8_t *ssid, uint32_t ssid_length,
                    uint8_t output[32])
{
    uint8_t message[36];
    for (uint32_t index = 0; index < ssid_length; ++index) message[index] = ssid[index];
    for (uint32_t block = 1; block <= 2u; ++block) {
        message[ssid_length] = 0;
        message[ssid_length + 1u] = 0;
        message[ssid_length + 2u] = 0;
        message[ssid_length + 3u] = (uint8_t)block;
        uint8_t u[20];
        uint8_t result[20];
        c6_hmac_sha1(passphrase, passphrase_length, message, ssid_length + 4u, u);
        for (uint32_t index = 0; index < 20u; ++index) result[index] = u[index];
        for (uint32_t round = 1; round < 4096u; ++round) {
            c6_hmac_sha1(passphrase, passphrase_length, u, sizeof(u), u);
            for (uint32_t index = 0; index < 20u; ++index) result[index] ^= u[index];
        }
        uint32_t take = block == 1u ? 20u : 12u;
        for (uint32_t index = 0; index < take; ++index) {
            output[(block - 1u) * 20u + index] = result[index];
        }
    }
}

static int ordered(const uint8_t *left, const uint8_t *right, uint32_t length)
{
    for (uint32_t index = 0; index < length; ++index) {
        if (left[index] < right[index]) return 1;
        if (left[index] > right[index]) return 0;
    }
    return 1;
}

void c6_wpa_prf(const uint8_t pmk[32], const uint8_t address_a[6],
                const uint8_t address_b[6], const uint8_t nonce_a[32],
                const uint8_t nonce_b[32], uint8_t ptk[64])
{
    static const uint8_t label[] = "Pairwise key expansion";
    uint8_t message[100];
    uint32_t offset = 0;
    for (uint32_t index = 0; index < sizeof(label) - 1u; ++index) message[offset++] = label[index];
    message[offset++] = 0;
    const uint8_t *first = ordered(address_a, address_b, 6) ? address_a : address_b;
    const uint8_t *second = first == address_a ? address_b : address_a;
    for (uint32_t index = 0; index < 6u; ++index) message[offset++] = first[index];
    for (uint32_t index = 0; index < 6u; ++index) message[offset++] = second[index];
    first = ordered(nonce_a, nonce_b, 32) ? nonce_a : nonce_b;
    second = first == nonce_a ? nonce_b : nonce_a;
    for (uint32_t index = 0; index < 32u; ++index) message[offset++] = first[index];
    for (uint32_t index = 0; index < 32u; ++index) message[offset++] = second[index];
    for (uint32_t block = 0; block < 4u; ++block) {
        message[offset] = (uint8_t)block;
        uint8_t digest[20];
        c6_hmac_sha1(pmk, 32, message, offset + 1u, digest);
        uint32_t take = block == 3u ? 4u : 20u;
        for (uint32_t index = 0; index < take; ++index) {
            ptk[block * 20u + index] = digest[index];
        }
    }
}
