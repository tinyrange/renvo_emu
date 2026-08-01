use super::*;

pub(super) fn kernel(id: usize, scenario: usize, input: &[u32; 16]) -> Kernel {
    let data = bytes(input);
    let length = 4 + scenario % 12;
    match id {
        0 => {
            let mut crc = 0xffff_ffff_u32 ^ input[15];
            for byte in &data[..length] {
                crc ^= u32::from(*byte);
                for _ in 0..8 {
                    crc = (crc >> 1) ^ (0xedb8_8320_u32 & 0_u32.wrapping_sub(crc & 1));
                }
            }
            simple(
                "crc32-bitwise",
                "checksums",
                crc ^ 0xffff_ffff,
                EMBENCH,
                format!(
                    "u32 crc = 0xffffffffu ^ input[15];\n\
                     for (u32 i = 0u; i < {length}u; ++i) {{\n\
                     \x20   crc ^= (u8)input[i];\n\
                     \x20   for (u32 bit = 0u; bit < 8u; ++bit)\n\
                     \x20       crc = (crc >> 1u) ^ (0xedb88320u & (0u - (crc & 1u)));\n\
                     }}\n\
                     return crc ^ 0xffffffffu;"
                ),
            )
        }
        1 => {
            let mut crc = (input[15] as u16) ^ 0xffff;
            for byte in &data[..length] {
                crc ^= u16::from(*byte) << 8;
                for _ in 0..8 {
                    crc = if crc & 0x8000 != 0 {
                        (crc << 1) ^ 0x1021
                    } else {
                        crc << 1
                    };
                }
            }
            simple(
                "crc16-ccitt",
                "checksums",
                u32::from(crc),
                EMBENCH,
                format!(
                    "u16 crc = (u16)input[15] ^ (u16)0xffffu;\n\
                     for (u32 i = 0u; i < {length}u; ++i) {{\n\
                     \x20   crc = (u16)(crc ^ ((u16)(u8)input[i] << 8u));\n\
                     \x20   for (u32 bit = 0u; bit < 8u; ++bit)\n\
                     \x20       crc = (crc & 0x8000u) ? (u16)((u16)(crc << 1u) ^ 0x1021u) : (u16)(crc << 1u);\n\
                     }}\n\
                     return (u32)crc;"
                ),
            )
        }
        2 => {
            let mut a = 0_u32;
            let mut b = 0_u32;
            for value in &input[..length] {
                a = (a + (value & 0xffff)) % 65_535;
                b = (b + a) % 65_535;
            }
            simple(
                "fletcher32-words",
                "checksums",
                (b << 16) | a,
                EMBENCH,
                format!(
                    "u32 a = 0u, b = 0u;\n\
                     for (u32 i = 0u; i < {length}u; ++i) {{\n\
                     \x20   a = (a + (input[i] & 0xffffu)) % 65535u;\n\
                     \x20   b = (b + a) % 65535u;\n\
                     }}\n\
                     return (b << 16u) | a;"
                ),
            )
        }
        3 => {
            let mut a = 1_u32;
            let mut b = 0_u32;
            for byte in &data[..length] {
                a = (a + u32::from(*byte)) % 65_521;
                b = (b + a) % 65_521;
            }
            simple(
                "adler32-bytes",
                "checksums",
                (b << 16) | a,
                EMBENCH,
                format!(
                    "u32 a = 1u, b = 0u;\n\
                     for (u32 i = 0u; i < {length}u; ++i) {{\n\
                     \x20   a = (a + (u8)input[i]) % 65521u;\n\
                     \x20   b = (b + a) % 65521u;\n\
                     }}\n\
                     return (b << 16u) | a;"
                ),
            )
        }
        4 => {
            let mut hash = 0x811c_9dc5_u32 ^ input[15];
            for byte in &data[..length] {
                hash = (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193);
            }
            simple(
                "fnv1a-stream",
                "hashing",
                hash,
                LLVM_TEST_SUITE,
                format!(
                    "u32 hash = 0x811c9dc5u ^ input[15];\n\
                     for (u32 i = 0u; i < {length}u; ++i)\n\
                     \x20   hash = (hash ^ (u8)input[i]) * 0x01000193u;\n\
                     return hash;"
                ),
            )
        }
        5 => {
            let expected = input[..length]
                .iter()
                .fold(0_u32, |sum, value| sum + value.count_ones());
            simple(
                "popcount-array",
                "bit-manipulation",
                expected,
                GCC_TORTURE,
                format!(
                    "u32 total = 0u;\n\
                     for (u32 i = 0u; i < {length}u; ++i) {{\n\
                     \x20   u32 x = input[i];\n\
                     \x20   while (x != 0u) {{ x &= x - 1u; ++total; }}\n\
                     }}\n\
                     return total;"
                ),
            )
        }
        6 => {
            let mut expected = 0_u32;
            for value in &input[..length] {
                let mut x = *value;
                x ^= x >> 16;
                x ^= x >> 8;
                x ^= x >> 4;
                expected = (expected << 1) | ((0x6996 >> (x & 15)) & 1);
            }
            simple(
                "parity-fold",
                "bit-manipulation",
                expected,
                GCC_TORTURE,
                format!(
                    "u32 result = 0u;\n\
                     for (u32 i = 0u; i < {length}u; ++i) {{\n\
                     \x20   u32 x = input[i];\n\
                     \x20   x ^= x >> 16u; x ^= x >> 8u; x ^= x >> 4u;\n\
                     \x20   result = (result << 1u) | ((0x6996u >> (x & 15u)) & 1u);\n\
                     }}\n\
                     return result;"
                ),
            )
        }
        7 => {
            let x = input[scenario % 16];
            let expected = x.reverse_bits() ^ x.rotate_left((scenario as u32 % 31) + 1);
            simple(
                "reverse-and-rotate",
                "bit-manipulation",
                expected,
                GCC_TORTURE,
                format!(
                    "u32 x = input[{}], reversed = 0u;\n\
                     for (u32 i = 0u; i < 32u; ++i) {{ reversed = (reversed << 1u) | (x & 1u); x >>= 1u; }}\n\
                     x = input[{}];\n\
                     return reversed ^ ((x << {}u) | (x >> {}u));",
                    scenario % 16,
                    scenario % 16,
                    scenario % 31 + 1,
                    32 - (scenario % 31 + 1)
                ),
            )
        }
        8 => {
            let x = input[scenario % 16];
            let mut decoded = x ^ (x >> 1);
            decoded ^= decoded >> 1;
            decoded ^= decoded >> 2;
            decoded ^= decoded >> 4;
            decoded ^= decoded >> 8;
            decoded ^= decoded >> 16;
            simple(
                "gray-code-roundtrip",
                "bit-manipulation",
                decoded,
                GCC_TORTURE,
                format!(
                    "u32 x = input[{}];\n\
                     u32 decoded = x ^ (x >> 1u);\n\
                     decoded ^= decoded >> 1u; decoded ^= decoded >> 2u;\n\
                     decoded ^= decoded >> 4u; decoded ^= decoded >> 8u; decoded ^= decoded >> 16u;\n\
                     return decoded;",
                    scenario % 16
                ),
            )
        }
        9 => {
            let a = input[0] & 0x1f;
            let b = input[1] & 0x3ff;
            let c = input[2] & 0x1ffff;
            let packed = a | (b << 5) | (c << 15);
            simple(
                "register-field-pack",
                "embedded-registers",
                ((packed >> 5) & 0x3ff) ^ ((packed >> 15) & 0x1ffff) ^ (packed & 0x1f),
                EMBENCH,
                "u32 packed = (input[0] & 0x1fu) | ((input[1] & 0x3ffu) << 5u) | ((input[2] & 0x1ffffu) << 15u);\n\
                 return (packed & 0x1fu) ^ ((packed >> 5u) & 0x3ffu) ^ ((packed >> 15u) & 0x1ffffu);"
                    .to_owned(),
            )
        }
        10 => {
            let mut values = input[..length].to_vec();
            values.sort_unstable();
            let expected = digest(&values);
            simple(
                "insertion-sort",
                "algorithms",
                expected,
                EMBENCH,
                format!(
                    "u32 a[16];\n\
                     for (u32 i = 0u; i < {length}u; ++i) a[i] = input[i];\n\
                     for (u32 i = 1u; i < {length}u; ++i) {{\n\
                     \x20   u32 value = a[i], j = i;\n\
                     \x20   while (j != 0u && a[j - 1u] > value) {{ a[j] = a[j - 1u]; --j; }}\n\
                     \x20   a[j] = value;\n\
                     }}\n\
                     u32 result = 0x9e3779b9u;\n\
                     for (u32 i = 0u; i < {length}u; ++i) result = (result << 5u) ^ (result >> 2u) ^ a[i];\n\
                     return result;"
                ),
            )
        }
        11 => {
            let mut values = input[..length].to_vec();
            values.sort_unstable();
            let needle = if scenario & 1 == 0 {
                values[scenario % length]
            } else {
                input[15] ^ 0xa5a5_a5a5
            };
            let expected = values
                .binary_search(&needle)
                .map_or(0xffff_ffff, |index| index as u32);
            let values_c = values
                .iter()
                .map(|value| format!("0x{value:08x}u"))
                .collect::<Vec<_>>()
                .join(", ");
            simple(
                "binary-search",
                "algorithms",
                expected,
                EMBENCH,
                format!(
                    "u32 a[{length}] = {{ {values_c} }};\n\
                     u32 needle = 0x{needle:08x}u, low = 0u, high = {length}u;\n\
                     while (low < high) {{\n\
                     \x20   u32 middle = low + ((high - low) >> 1u);\n\
                     \x20   if (a[middle] < needle) low = middle + 1u; else high = middle;\n\
                     }}\n\
                     return (low < {length}u && a[low] == needle) ? low : 0xffffffffu;"
                ),
            )
        }
        12 => ring_buffer_kernel(input, scenario),
        13 => {
            let mut bins = [0_u32; 8];
            for byte in &data[..length] {
                bins[usize::from(*byte & 7)] += 1;
            }
            simple(
                "byte-histogram",
                "data-layout",
                digest(&bins),
                LLVM_TEST_SUITE,
                format!(
                    "u32 bins[8] = {{ 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u }};\n\
                     for (u32 i = 0u; i < {length}u; ++i) ++bins[(u8)input[i] & 7u];\n\
                     u32 result = 0x9e3779b9u;\n\
                     for (u32 i = 0u; i < 8u; ++i) result = (result << 5u) ^ (result >> 2u) ^ bins[i];\n\
                     return result;"
                ),
            )
        }
        14 => {
            let mut out = [0_u32; 9];
            for row in 0..3 {
                for column in 0..3 {
                    for k in 0..3 {
                        out[row * 3 + column] = out[row * 3 + column].wrapping_add(
                            (input[row * 3 + k] & 0xff)
                                .wrapping_mul(input[9 + k * 2 + column % 2] & 0xff),
                        );
                    }
                }
            }
            simple(
                "matrix3-multiply",
                "algorithms",
                digest(&out),
                EMBENCH,
                "u32 out[9] = { 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u };\n\
                 for (u32 row = 0u; row < 3u; ++row)\n\
                 \x20   for (u32 column = 0u; column < 3u; ++column)\n\
                 \x20       for (u32 k = 0u; k < 3u; ++k)\n\
                 \x20           out[row * 3u + column] += (input[row * 3u + k] & 0xffu) * (input[9u + k * 2u + column % 2u] & 0xffu);\n\
                 u32 result = 0x9e3779b9u;\n\
                 for (u32 i = 0u; i < 9u; ++i) result = (result << 5u) ^ (result >> 2u) ^ out[i];\n\
                 return result;"
                    .to_owned(),
            )
        }
        15 => {
            let x = input[scenario % 16];
            let swapped = x.swap_bytes();
            simple(
                "endian-load-store",
                "data-layout",
                swapped ^ x.rotate_left(8),
                LLVM_TEST_SUITE,
                format!(
                    "u32 x = input[{}];\n\
                     u8 bytes[4];\n\
                     bytes[0] = (u8)(x >> 24u); bytes[1] = (u8)(x >> 16u);\n\
                     bytes[2] = (u8)(x >> 8u); bytes[3] = (u8)x;\n\
                     u32 swapped = (u32)bytes[0] | ((u32)bytes[1] << 8u) | ((u32)bytes[2] << 16u) | ((u32)bytes[3] << 24u);\n\
                     return swapped ^ ((x << 8u) | (x >> 24u));",
                    scenario % 16
                ),
            )
        }
        16 => overlap_move_kernel(input, scenario),
        17 => {
            let selector = input[0] % 12;
            let expected = dense_switch(selector, input);
            simple(
                "dense-switch",
                "control-flow",
                expected,
                GCC_TORTURE,
                "switch (input[0] % 12u) {\n\
                 case 0u: return input[1] + input[2]; case 1u: return input[1] - input[2];\n\
                 case 2u: return input[1] ^ input[2]; case 3u: return input[1] | input[2];\n\
                 case 4u: return input[1] & input[2]; case 5u: return input[1] * 3u;\n\
                 case 6u: return input[2] * 5u; case 7u: return input[3] >> 3u;\n\
                 case 8u: return input[4] << 7u; case 9u: return ~input[5];\n\
                 case 10u: return input[6] + 0x1234u; default: return input[7] ^ 0xa5a5a5a5u;\n\
                 }"
                .to_owned(),
            )
        }
        18 => {
            let selector = input[0] & 0xffff;
            let expected = match selector {
                1 => input[1],
                17 => input[2],
                257 => input[3],
                4093 => input[4],
                32_749 => input[5],
                65_535 => input[6],
                _ => input[7] ^ selector,
            };
            simple(
                "sparse-switch",
                "control-flow",
                expected,
                GCC_TORTURE,
                "u32 selector = input[0] & 0xffffu;\n\
                 switch (selector) {\n\
                 case 1u: return input[1]; case 17u: return input[2]; case 257u: return input[3];\n\
                 case 4093u: return input[4]; case 32749u: return input[5]; case 65535u: return input[6];\n\
                 default: return input[7] ^ selector;\n\
                 }"
                .to_owned(),
            )
        }
        19 => state_machine_kernel(input, length),
        20 => short_circuit_kernel(input, scenario),
        21 => nested_break_kernel(input, scenario),
        22 => abi_eight_args_kernel(input),
        23 => struct_by_value_kernel(input),
        24 => struct_return_kernel(input),
        25 => function_pointer_kernel(input, scenario),
        26 => {
            let a = (input[0] % 60_000) + 1;
            let b = (input[1] % 60_000) + 1;
            let expected = gcd(a, b);
            simple_with_helpers(
                "recursive-gcd",
                "abi-calls",
                expected,
                GCC_TORTURE,
                "__attribute__((noinline)) static u32 gcd(u32 a, u32 b)\n\
                 {\n\
                 \x20   return (b == 0u) ? a : gcd(b, a % b);\n\
                 }\n"
                .to_owned(),
                format!("return gcd({a}u, {b}u);"),
            )
        }
        27 => {
            let now = input[0];
            let deadline = now.wrapping_add(input[1] & 0xffff);
            let observed = deadline.wrapping_add(input[2] & 0x1ffff);
            let expired = observed.wrapping_sub(deadline) < 0x8000_0000;
            simple(
                "timer-wrap-deadline",
                "embedded-control",
                u32::from(expired) ^ observed,
                EMBENCH,
                "u32 now = input[0];\n\
                 u32 deadline = now + (input[1] & 0xffffu);\n\
                 u32 observed = deadline + (input[2] & 0x1ffffu);\n\
                 u32 expired = (observed - deadline) < 0x80000000u;\n\
                 return expired ^ observed;"
                    .to_owned(),
            )
        }
        28 => debounce_kernel(input, length),
        29 => uart_frame_kernel(input, scenario),
        30 => cobs_kernel(&data, length),
        31 => spi_shift_kernel(input, scenario),
        32 => crc8_kernel(&data, length),
        33 => saturation_kernel(input),
        34 => fixed_point_kernel(input, length),
        35 => integer_sqrt_kernel(input[scenario % 16]),
        36 => restoring_division_kernel(input),
        37 => {
            let a = input[0] & 0xffff;
            let b = input[1] & 0xffff;
            let product = a * b;
            simple(
                "multiply-high16",
                "integer-arithmetic",
                ((product >> 16) << 16) | (product & 0xffff),
                GCC_TORTURE,
                "u32 a = input[0] & 0xffffu, b = input[1] & 0xffffu;\n\
                 u32 product = a * b;\n\
                 return ((product >> 16u) << 16u) | (product & 0xffffu);"
                    .to_owned(),
            )
        }
        38 => fir_kernel(input, length),
        39 => median5_kernel(input, scenario),
        _ => unreachable!(),
    }
}

fn simple(
    slug: &'static str,
    category: &'static str,
    expected: u32,
    inspiration: &'static str,
    body: String,
) -> Kernel {
    simple_with_helpers(slug, category, expected, inspiration, String::new(), body)
}

fn simple_with_helpers(
    slug: &'static str,
    category: &'static str,
    expected: u32,
    inspiration: &'static str,
    helpers: String,
    body: String,
) -> Kernel {
    Kernel {
        slug,
        category,
        expected,
        inspiration,
        helpers,
        body,
    }
}

pub(super) fn digest(values: &[u32]) -> u32 {
    values.iter().fold(0x9e37_79b9_u32, |result, value| {
        (result << 5) ^ (result >> 2) ^ value
    })
}

fn ring_buffer_kernel(input: &[u32; 16], scenario: usize) -> Kernel {
    let capacity = 5 + scenario % 7;
    let operations = 12 + scenario % 4;
    let mut queue = vec![0_u32; capacity];
    let (mut head, mut tail, mut count, mut result) = (0, 0, 0, 0_u32);
    for (i, value) in input.iter().cycle().take(operations).enumerate() {
        if (value >> (i % 17)) & 1 != 0 && count != 0 {
            result = result.rotate_left(3) ^ queue[head];
            head = (head + 1) % capacity;
            count -= 1;
        } else if count != capacity {
            queue[tail] = *value ^ i as u32;
            tail = (tail + 1) % capacity;
            count += 1;
        }
    }
    while count != 0 {
        result = result.rotate_left(3) ^ queue[head];
        head = (head + 1) % capacity;
        count -= 1;
    }
    simple(
        "ring-buffer",
        "data-structures",
        result,
        EMBENCH,
        format!(
            "u32 queue[{capacity}], head = 0u, tail = 0u, count = 0u, result = 0u;\n\
             for (u32 i = 0u; i < {operations}u; ++i) {{\n\
             \x20   u32 value = input[i & 15u];\n\
             \x20   if (((value >> (i % 17u)) & 1u) != 0u && count != 0u) {{\n\
             \x20       result = ((result << 3u) | (result >> 29u)) ^ queue[head]; head = (head + 1u) % {capacity}u; --count;\n\
             \x20   }} else if (count != {capacity}u) {{ queue[tail] = value ^ i; tail = (tail + 1u) % {capacity}u; ++count; }}\n\
             }}\n\
             while (count != 0u) {{ result = ((result << 3u) | (result >> 29u)) ^ queue[head]; head = (head + 1u) % {capacity}u; --count; }}\n\
             return result;"
        ),
    )
}

fn overlap_move_kernel(input: &[u32; 16], scenario: usize) -> Kernel {
    let source = scenario % 5;
    let destination = 5 + scenario % 4;
    let count = 4 + scenario % 4;
    let mut values = *input;
    if destination > source {
        for i in (0..count).rev() {
            values[destination + i] = values[source + i];
        }
    } else {
        for i in 0..count {
            values[destination + i] = values[source + i];
        }
    }
    simple(
        "overlap-move",
        "memory",
        digest(&values),
        LLVM_TEST_SUITE,
        format!(
            "u32 a[16];\n\
             for (u32 i = 0u; i < 16u; ++i) a[i] = input[i];\n\
             u32 source = {source}u, destination = {destination}u, count = {count}u;\n\
             if (destination > source) {{\n\
             \x20   while (count != 0u) {{ --count; a[destination + count] = a[source + count]; }}\n\
             }} else {{ for (u32 i = 0u; i < count; ++i) a[destination + i] = a[source + i]; }}\n\
             u32 result = 0x9e3779b9u;\n\
             for (u32 i = 0u; i < 16u; ++i) result = (result << 5u) ^ (result >> 2u) ^ a[i];\n\
             return result;"
        ),
    )
}

fn dense_switch(selector: u32, input: &[u32; 16]) -> u32 {
    match selector {
        0 => input[1].wrapping_add(input[2]),
        1 => input[1].wrapping_sub(input[2]),
        2 => input[1] ^ input[2],
        3 => input[1] | input[2],
        4 => input[1] & input[2],
        5 => input[1].wrapping_mul(3),
        6 => input[2].wrapping_mul(5),
        7 => input[3] >> 3,
        8 => input[4] << 7,
        9 => !input[5],
        10 => input[6].wrapping_add(0x1234),
        _ => input[7] ^ 0xa5a5_a5a5,
    }
}

fn state_machine_kernel(input: &[u32; 16], length: usize) -> Kernel {
    let mut state = 0_u32;
    let mut accepted = 0_u32;
    for value in &input[..length] {
        let symbol = value & 3;
        state = match (state, symbol) {
            (1, 2) => 2,
            (2, 3) => {
                accepted += 1;
                0
            }
            (_, 1) => 1,
            _ => 0,
        };
    }
    simple(
        "protocol-state-machine",
        "control-flow",
        (accepted << 16) | state,
        EMBENCH,
        format!(
            "u32 state = 0u, accepted = 0u;\n\
             for (u32 i = 0u; i < {length}u; ++i) {{\n\
             \x20   u32 symbol = input[i] & 3u;\n\
             \x20   if (state == 0u && symbol == 1u) state = 1u;\n\
             \x20   else if (state == 1u && symbol == 2u) state = 2u;\n\
             \x20   else if (state == 2u && symbol == 3u) {{ ++accepted; state = 0u; }}\n\
             \x20   else if (symbol == 1u) state = 1u; else state = 0u;\n\
             }}\n\
             return (accepted << 16u) | state;"
        ),
    )
}

fn short_circuit_kernel(input: &[u32; 16], scenario: usize) -> Kernel {
    let mut effects = 0_u32;
    let left = input[0] & (1 << (scenario % 31));
    if left != 0 && {
        effects += 3;
        input[1] & 1 != 0
    } {
        effects += 5;
    }
    if left != 0 || {
        effects += 7;
        input[2] & 1 != 0
    } {
        effects += 11;
    }
    simple(
        "short-circuit-effects",
        "optimizer-control",
        effects,
        CSMITH,
        format!(
            "u32 effects = 0u, left = input[0] & (1u << {}u);\n\
             if (left != 0u && (effects += 3u, (input[1] & 1u) != 0u)) effects += 5u;\n\
             if (left != 0u || (effects += 7u, (input[2] & 1u) != 0u)) effects += 11u;\n\
             return effects;",
            scenario % 31
        ),
    )
}

fn nested_break_kernel(input: &[u32; 16], scenario: usize) -> Kernel {
    let limit = 3 + scenario % 6;
    let mut result = 0_u32;
    'outer: for i in 0..limit {
        for j in 0..limit {
            let value = input[(i * limit + j) & 15];
            if value & 0xff == scenario as u32 {
                result ^= 0x8000_0000 | ((i as u32) << 8) | j as u32;
                break 'outer;
            }
            if value.trailing_zeros() >= 3 {
                continue;
            }
            result = result.rotate_left(1) ^ value;
        }
    }
    simple(
        "nested-break-continue",
        "control-flow",
        result,
        GCC_TORTURE,
        format!(
            "u32 result = 0u, stop = 0u;\n\
             for (u32 i = 0u; i < {limit}u && stop == 0u; ++i) {{\n\
             \x20   for (u32 j = 0u; j < {limit}u; ++j) {{\n\
             \x20       u32 value = input[(i * {limit}u + j) & 15u];\n\
             \x20       if ((value & 0xffu) == {scenario}u) {{ result ^= 0x80000000u | (i << 8u) | j; stop = 1u; break; }}\n\
             \x20       if ((value & 7u) == 0u) continue;\n\
             \x20       result = ((result << 1u) | (result >> 31u)) ^ value;\n\
             \x20   }}\n\
             }}\n\
             return result;"
        ),
    )
}

fn abi_eight_args_kernel(input: &[u32; 16]) -> Kernel {
    fn mix(args: &[u32]) -> u32 {
        args.iter().enumerate().fold(0_u32, |result, (i, value)| {
            result.wrapping_add(value.rotate_left((i * 3 + 1) as u32))
        })
    }
    simple_with_helpers(
        "eight-argument-call",
        "abi-calls",
        mix(&input[..8]) ^ mix(&input[8..]),
        GCC_TORTURE,
        "__attribute__((noinline)) static u32 mix8(u32 a, u32 b, u32 c, u32 d, u32 e, u32 f, u32 g, u32 h)\n\
         {\n\
         \x20   return (a << 1u | a >> 31u) + (b << 4u | b >> 28u) + (c << 7u | c >> 25u) + (d << 10u | d >> 22u)\n\
         \x20       + (e << 13u | e >> 19u) + (f << 16u | f >> 16u) + (g << 19u | g >> 13u) + (h << 22u | h >> 10u);\n\
         }\n"
            .to_owned(),
        "return mix8(input[0], input[1], input[2], input[3], input[4], input[5], input[6], input[7])\n\
         \x20   ^ mix8(input[8], input[9], input[10], input[11], input[12], input[13], input[14], input[15]);"
            .to_owned(),
    )
}

fn struct_by_value_kernel(input: &[u32; 16]) -> Kernel {
    let length = u32::from(input[1] as u16);
    let flags = u32::from(input[2] as u8);
    let expected = input[0].wrapping_add(input[3].rotate_left(5))
        ^ length.wrapping_mul(3)
        ^ flags.rotate_right(7);
    simple_with_helpers(
        "struct-by-value",
        "abi-aggregates",
        expected,
        GCC_TORTURE,
        "struct packet { u32 tag; u16 length; u8 flags; u8 sequence; u32 payload; };\n\
         __attribute__((noinline)) static u32 consume(struct packet p)\n\
         {\n\
         \x20   return p.tag + (p.payload << 5u | p.payload >> 27u) ^ (u32)p.length * 3u ^ ((u32)p.flags << 25u | (u32)p.flags >> 7u);\n\
         }\n"
            .to_owned(),
        "struct packet p = { input[0], (u16)input[1], (u8)input[2], (u8)input[3], input[3] };\n\
         return consume(p);"
            .to_owned(),
    )
}

fn struct_return_kernel(input: &[u32; 16]) -> Kernel {
    let first = input[0].wrapping_add(input[2]);
    let second = input[1].wrapping_sub(input[3]);
    let third = input[0] ^ input[1] ^ input[2] ^ input[3];
    simple_with_helpers(
        "struct-return",
        "abi-aggregates",
        first ^ second.rotate_left(9) ^ third.rotate_right(11),
        GCC_TORTURE,
        "struct triple { u32 first; u32 second; u32 third; };\n\
         __attribute__((noinline)) static struct triple make_triple(u32 a, u32 b, u32 c, u32 d)\n\
         {\n\
         \x20   struct triple result = { a + c, b - d, a ^ b ^ c ^ d };\n\
         \x20   return result;\n\
         }\n"
            .to_owned(),
        "struct triple value = make_triple(input[0], input[1], input[2], input[3]);\n\
         return value.first ^ (value.second << 9u | value.second >> 23u) ^ (value.third >> 11u | value.third << 21u);"
            .to_owned(),
    )
}

fn function_pointer_kernel(input: &[u32; 16], scenario: usize) -> Kernel {
    let op = scenario % 4;
    let expected = match op {
        0 => input[0].wrapping_add(input[1]),
        1 => input[0].wrapping_sub(input[1]),
        2 => input[0] ^ input[1],
        _ => input[0].rotate_left(input[1] & 31),
    };
    simple_with_helpers(
        "function-pointer-dispatch",
        "abi-calls",
        expected,
        LLVM_TEST_SUITE,
        "typedef u32 (*operation)(u32, u32);\n\
         __attribute__((noinline)) static u32 add(u32 a, u32 b) { return a + b; }\n\
         __attribute__((noinline)) static u32 sub(u32 a, u32 b) { return a - b; }\n\
         __attribute__((noinline)) static u32 xor(u32 a, u32 b) { return a ^ b; }\n\
         __attribute__((noinline)) static u32 rol(u32 a, u32 b) { b &= 31u; return b == 0u ? a : (a << b) | (a >> (32u - b)); }\n"
            .to_owned(),
        format!(
            "operation table[4] = {{ add, sub, xor, rol }};\n\
             operation selected = table[{op}u];\n\
             return selected(input[0], input[1]);"
        ),
    )
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

fn debounce_kernel(input: &[u32; 16], length: usize) -> Kernel {
    let threshold = 2 + length % 4;
    let mut stable = input[0] & 1;
    let mut candidate = stable;
    let mut count = 0;
    let mut transitions = 0;
    for value in &input[..length] {
        let sample = value & 1;
        if sample == candidate {
            count += 1;
        } else {
            candidate = sample;
            count = 1;
        }
        if count >= threshold && stable != candidate {
            stable = candidate;
            transitions += 1;
        }
    }
    simple(
        "gpio-debounce",
        "embedded-control",
        (transitions << 16) | stable,
        EMBENCH,
        format!(
            "u32 stable = input[0] & 1u, candidate = stable, count = 0u, transitions = 0u;\n\
             for (u32 i = 0u; i < {length}u; ++i) {{\n\
             \x20   u32 sample = input[i] & 1u;\n\
             \x20   if (sample == candidate) ++count; else {{ candidate = sample; count = 1u; }}\n\
             \x20   if (count >= {threshold}u && stable != candidate) {{ stable = candidate; ++transitions; }}\n\
             }}\n\
             return (transitions << 16u) | stable;"
        ),
    )
}

fn uart_frame_kernel(input: &[u32; 16], scenario: usize) -> Kernel {
    let byte = input[scenario % 16] & 0xff;
    let parity = byte.count_ones() & 1;
    let frame = 1_u32 << 9 | parity << 9 | byte << 1;
    simple(
        "uart-even-parity-frame",
        "serial-protocols",
        frame ^ frame.reverse_bits(),
        EMBENCH,
        format!(
            "u32 byte = input[{}] & 0xffu;\n\
             u32 parity = byte, reversed = 0u;\n\
             parity ^= parity >> 4u; parity ^= parity >> 2u; parity ^= parity >> 1u; parity &= 1u;\n\
             u32 frame = (1u << 9u) | (parity << 9u) | (byte << 1u);\n\
             u32 copy = frame;\n\
             for (u32 i = 0u; i < 32u; ++i) {{ reversed = (reversed << 1u) | (copy & 1u); copy >>= 1u; }}\n\
             return frame ^ reversed;",
            scenario % 16
        ),
    )
}

fn cobs_kernel(data: &[u8; 16], length: usize) -> Kernel {
    let mut output = [0_u32; 18];
    let mut read = 0;
    let mut write = 1;
    let mut code_index = 0;
    let mut code = 1_u32;
    while read < length {
        if data[read] == 0 {
            output[code_index] = code;
            code = 1;
            code_index = write;
            write += 1;
        } else {
            output[write] = u32::from(data[read]);
            write += 1;
            code += 1;
        }
        read += 1;
    }
    output[code_index] = code;
    simple(
        "cobs-encode",
        "serial-protocols",
        digest(&output[..write]) ^ write as u32,
        EMBENCH,
        format!(
            "u32 output[18], read = 0u, write = 1u, code_index = 0u, code = 1u;\n\
             while (read < {length}u) {{\n\
             \x20   u32 byte = (u8)input[read];\n\
             \x20   if (byte == 0u) {{ output[code_index] = code; code = 1u; code_index = write++; }}\n\
             \x20   else {{ output[write++] = byte; ++code; }}\n\
             \x20   ++read;\n\
             }}\n\
             output[code_index] = code;\n\
             u32 result = 0x9e3779b9u;\n\
             for (u32 i = 0u; i < write; ++i) result = (result << 5u) ^ (result >> 2u) ^ output[i];\n\
             return result ^ write;"
        ),
    )
}

fn spi_shift_kernel(input: &[u32; 16], scenario: usize) -> Kernel {
    let tx = input[0] as u8;
    let rx_pattern = input[1] as u8;
    let lsb_first = scenario & 1 != 0;
    let mut rx = 0_u32;
    let mut pins = 0_u32;
    for bit in 0..8 {
        let shift = if lsb_first { bit } else { 7 - bit };
        let mosi = (u32::from(tx) >> shift) & 1;
        let miso = (u32::from(rx_pattern) >> (7 - bit)) & 1;
        pins = pins.rotate_left(3) ^ (mosi | (miso << 1));
        rx = (rx << 1) | miso;
    }
    simple(
        "spi-shift-register",
        "serial-protocols",
        (rx << 24) ^ pins,
        EMBENCH,
        format!(
            "u32 tx = (u8)input[0], pattern = (u8)input[1], rx = 0u, pins = 0u;\n\
             for (u32 bit = 0u; bit < 8u; ++bit) {{\n\
             \x20   u32 shift = {};\n\
             \x20   u32 mosi = (tx >> shift) & 1u, miso = (pattern >> (7u - bit)) & 1u;\n\
             \x20   pins = ((pins << 3u) | (pins >> 29u)) ^ (mosi | (miso << 1u));\n\
             \x20   rx = (rx << 1u) | miso;\n\
             }}\n\
             return (rx << 24u) ^ pins;",
            if lsb_first { "bit" } else { "7u - bit" }
        ),
    )
}

fn crc8_kernel(data: &[u8; 16], length: usize) -> Kernel {
    let mut crc = 0_u8;
    for byte in &data[..length] {
        crc ^= *byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x07
            } else {
                crc << 1
            };
        }
    }
    simple(
        "crc8-sensor-packet",
        "serial-protocols",
        u32::from(crc),
        EMBENCH,
        format!(
            "u8 crc = 0u;\n\
             for (u32 i = 0u; i < {length}u; ++i) {{\n\
             \x20   crc = (u8)(crc ^ (u8)input[i]);\n\
             \x20   for (u32 bit = 0u; bit < 8u; ++bit) crc = (crc & 0x80u) ? (u8)((u8)(crc << 1u) ^ 0x07u) : (u8)(crc << 1u);\n\
             }}\n\
             return (u32)crc;"
        ),
    )
}

fn saturation_kernel(input: &[u32; 16]) -> Kernel {
    let a = input[0] & 0xffff;
    let b = input[1] & 0xffff;
    let sum = (a + b).min(0xffff);
    let difference = a.saturating_sub(b);
    simple(
        "adc-saturating-arithmetic",
        "fixed-point-dsp",
        (sum << 16) | difference,
        EMBENCH,
        "u32 a = input[0] & 0xffffu, b = input[1] & 0xffffu;\n\
         u32 sum = a + b;\n\
         if (sum > 0xffffu) sum = 0xffffu;\n\
         u32 difference = (a < b) ? 0u : a - b;\n\
         return (sum << 16u) | difference;"
            .to_owned(),
    )
}

fn fixed_point_kernel(input: &[u32; 16], length: usize) -> Kernel {
    let mut accumulator = 0_u32;
    for i in 0..length {
        let sample = input[i] & 0xffff;
        let gain = (input[15 - i] & 0x7fff) + 1;
        accumulator = accumulator.wrapping_add(sample * gain);
    }
    simple(
        "q16-multiply-accumulate",
        "fixed-point-dsp",
        (accumulator >> 16) ^ accumulator,
        EMBENCH,
        format!(
            "u32 accumulator = 0u;\n\
             for (u32 i = 0u; i < {length}u; ++i) {{\n\
             \x20   u32 sample = input[i] & 0xffffu, gain = (input[15u - i] & 0x7fffu) + 1u;\n\
             \x20   accumulator += sample * gain;\n\
             }}\n\
             return (accumulator >> 16u) ^ accumulator;"
        ),
    )
}

fn integer_sqrt_kernel(value: u32) -> Kernel {
    let mut remainder = value;
    let mut root = 0_u32;
    let mut bit = 1_u32 << 30;
    while bit > remainder {
        bit >>= 2;
    }
    while bit != 0 {
        if remainder >= root + bit {
            remainder -= root + bit;
            root = (root >> 1) + bit;
        } else {
            root >>= 1;
        }
        bit >>= 2;
    }
    simple(
        "integer-square-root",
        "integer-arithmetic",
        root ^ remainder,
        EMBENCH,
        format!(
            "u32 remainder = 0x{value:08x}u, root = 0u, bit = 1u << 30u;\n\
             while (bit > remainder) bit >>= 2u;\n\
             while (bit != 0u) {{\n\
             \x20   if (remainder >= root + bit) {{ remainder -= root + bit; root = (root >> 1u) + bit; }} else root >>= 1u;\n\
             \x20   bit >>= 2u;\n\
             }}\n\
             return root ^ remainder;"
        ),
    )
}

fn restoring_division_kernel(input: &[u32; 16]) -> Kernel {
    let numerator = input[0];
    let divisor = (input[1] & 0xffff) | 1;
    let expected = (numerator / divisor) ^ (numerator % divisor).rotate_left(13);
    simple(
        "restoring-division",
        "integer-arithmetic",
        expected,
        GCC_TORTURE,
        "u32 numerator = input[0], divisor = (input[1] & 0xffffu) | 1u;\n\
         u32 quotient = 0u, remainder = 0u;\n\
         for (u32 bit = 32u; bit != 0u; --bit) {\n\
         \x20   remainder = (remainder << 1u) | ((numerator >> (bit - 1u)) & 1u);\n\
         \x20   if (remainder >= divisor) { remainder -= divisor; quotient |= 1u << (bit - 1u); }\n\
         }\n\
         return quotient ^ ((remainder << 13u) | (remainder >> 19u));"
            .to_owned(),
    )
}

fn fir_kernel(input: &[u32; 16], length: usize) -> Kernel {
    const COEFFICIENTS: [u32; 5] = [3, 5, 7, 5, 3];
    let mut result = 0_u32;
    for i in 4..length {
        let mut sample = 0_u32;
        for tap in 0..5 {
            sample = sample.wrapping_add((input[i - tap] & 0xff) * COEFFICIENTS[tap]);
        }
        result = result.rotate_left(3) ^ sample;
    }
    simple(
        "fir-five-tap",
        "fixed-point-dsp",
        result,
        EMBENCH,
        format!(
            "const u32 coefficients[5] = {{ 3u, 5u, 7u, 5u, 3u }};\n\
             u32 result = 0u;\n\
             for (u32 i = 4u; i < {length}u; ++i) {{\n\
             \x20   u32 sample = 0u;\n\
             \x20   for (u32 tap = 0u; tap < 5u; ++tap) sample += (input[i - tap] & 0xffu) * coefficients[tap];\n\
             \x20   result = ((result << 3u) | (result >> 29u)) ^ sample;\n\
             }}\n\
             return result;"
        ),
    )
}

fn median5_kernel(input: &[u32; 16], scenario: usize) -> Kernel {
    let base = scenario % 12;
    let mut values = [
        input[base] & 0xffff,
        input[base + 1] & 0xffff,
        input[base + 2] & 0xffff,
        input[base + 3] & 0xffff,
        input[(base + 4) & 15] & 0xffff,
    ];
    values.sort_unstable();
    simple(
        "median-five",
        "fixed-point-dsp",
        values[2] ^ values[0].rotate_left(16) ^ values[4],
        EMBENCH,
        format!(
            "u32 a[5] = {{ input[{base}] & 0xffffu, input[{}] & 0xffffu, input[{}] & 0xffffu, input[{}] & 0xffffu, input[{}] & 0xffffu }};\n\
             for (u32 i = 1u; i < 5u; ++i) {{ u32 value = a[i], j = i; while (j != 0u && a[j - 1u] > value) {{ a[j] = a[j - 1u]; --j; }} a[j] = value; }}\n\
             return a[2] ^ (a[0] << 16u | a[0] >> 16u) ^ a[4];",
            base + 1,
            base + 2,
            base + 3,
            (base + 4) & 15
        ),
    )
}
