pub(super) fn execute_m(funct3: u32, left: u32, right: u32) -> Option<u32> {
    Some(match funct3 {
        0 => left.wrapping_mul(right),
        1 => (((left as i32 as i64) * (right as i32 as i64)) >> 32) as u32,
        2 => (((left as i32 as i64) * i64::from(right)) >> 32) as u32,
        3 => ((u64::from(left) * u64::from(right)) >> 32) as u32,
        4 => {
            if right == 0 {
                u32::MAX
            } else if left == 0x8000_0000 && right == u32::MAX {
                left
            } else {
                ((left as i32) / (right as i32)) as u32
            }
        }
        5 => {
            if right == 0 {
                u32::MAX
            } else {
                left / right
            }
        }
        6 => {
            if right == 0 {
                left
            } else if left == 0x8000_0000 && right == u32::MAX {
                0
            } else {
                ((left as i32) % (right as i32)) as u32
            }
        }
        7 => {
            if right == 0 {
                left
            } else {
                left % right
            }
        }
        _ => return None,
    })
}

pub(super) fn execute_b_register(funct7: u32, funct3: u32, left: u32, right: u32) -> Option<u32> {
    let shift = right & 0x1f;
    Some(match (funct7, funct3) {
        // Zba
        (0x10, 2) => right.wrapping_add(left << 1),
        (0x10, 4) => right.wrapping_add(left << 2),
        (0x10, 6) => right.wrapping_add(left << 3),
        // Zbb
        (0x20, 4) => left ^ !right,
        (0x20, 6) => left | !right,
        (0x20, 7) => left & !right,
        (0x30, 1) => left.rotate_left(shift),
        (0x30, 5) => left.rotate_right(shift),
        (0x05, 4) => (left as i32).min(right as i32) as u32,
        (0x05, 5) => left.min(right),
        (0x05, 6) => (left as i32).max(right as i32) as u32,
        (0x05, 7) => left.max(right),
        // Zbkb
        (0x04, 4) => (left & 0xffff) | (right << 16),
        (0x04, 7) => (left & 0xff) | ((right & 0xff) << 8),
        // Zbs
        (0x24, 1) => left & !(1 << shift),
        (0x24, 5) => (left >> shift) & 1,
        (0x34, 1) => left ^ (1 << shift),
        (0x14, 1) => left | (1 << shift),
        _ => return None,
    })
}

pub(super) fn execute_b_immediate(
    instruction: u32,
    funct3: u32,
    left: u32,
    shift_register: u8,
) -> Option<u32> {
    let immediate = instruction >> 20;
    let funct7 = instruction >> 25;
    let shift = u32::from(shift_register & 0x1f);
    Some(match (funct3, immediate) {
        (1, 0x600) => left.leading_zeros(),
        (1, 0x601) => left.trailing_zeros(),
        (1, 0x602) => left.count_ones(),
        (1, 0x604) => i32::from(left as u8 as i8) as u32,
        (1, 0x605) => i32::from(left as u16 as i16) as u32,
        (5, 0x698) => left.swap_bytes(),
        (5, 0x287) => {
            let mut result = 0_u32;
            for byte in 0..4 {
                if left & (0xff << (byte * 8)) != 0 {
                    result |= 0xff << (byte * 8);
                }
            }
            result
        }
        _ => match (funct7, funct3) {
            (0x30, 5) => left.rotate_right(shift),
            (0x24, 1) => left & !(1 << shift),
            (0x24, 5) => (left >> shift) & 1,
            (0x34, 1) => left ^ (1 << shift),
            (0x14, 1) => left | (1 << shift),
            _ => return None,
        },
    })
}

pub(super) const fn sign_extend(value: u32, bits: u32) -> i32 {
    let shift = 32 - bits;
    ((value << shift) as i32) >> shift
}

pub(super) const fn decode_j_immediate(instruction: u32) -> i32 {
    let encoded = ((instruction >> 31) << 20)
        | (((instruction >> 12) & 0xff) << 12)
        | (((instruction >> 20) & 1) << 11)
        | (((instruction >> 21) & 0x3ff) << 1);
    sign_extend(encoded, 21)
}

pub(super) const fn decode_b_immediate(instruction: u32) -> i32 {
    let encoded = ((instruction >> 31) << 12)
        | (((instruction >> 7) & 1) << 11)
        | (((instruction >> 25) & 0x3f) << 5)
        | (((instruction >> 8) & 0xf) << 1);
    sign_extend(encoded, 13)
}

pub(super) const fn compact_register(encoded: u16) -> u8 {
    8 + encoded as u8
}

pub(super) fn decode_c_imm6(instruction: u16) -> i32 {
    sign_extend(
        u32::from((instruction >> 2) & 0x1f) | (u32::from(instruction >> 12) << 5),
        6,
    )
}

pub(super) fn decode_c_addi4spn(instruction: u16) -> u32 {
    (u32::from((instruction >> 6) & 1) << 2)
        | (u32::from((instruction >> 5) & 1) << 3)
        | (u32::from((instruction >> 11) & 0x3) << 4)
        | (u32::from((instruction >> 7) & 0xf) << 6)
}

pub(super) fn decode_c_lw_sw(instruction: u16) -> u32 {
    (u32::from((instruction >> 6) & 1) << 2)
        | (u32::from((instruction >> 10) & 0x7) << 3)
        | (u32::from((instruction >> 5) & 1) << 6)
}

pub(super) fn decode_c_lwsp(instruction: u16) -> u32 {
    (u32::from((instruction >> 4) & 0x7) << 2)
        | (u32::from((instruction >> 12) & 1) << 5)
        | (u32::from((instruction >> 2) & 0x3) << 6)
}

pub(super) fn decode_c_swsp(instruction: u16) -> u32 {
    (u32::from((instruction >> 9) & 0xf) << 2) | (u32::from((instruction >> 7) & 0x3) << 6)
}

pub(super) fn decode_c_addi16sp(instruction: u16) -> i32 {
    let encoded = (u32::from((instruction >> 6) & 1) << 4)
        | (u32::from((instruction >> 2) & 1) << 5)
        | (u32::from((instruction >> 5) & 1) << 6)
        | (u32::from((instruction >> 3) & 0x3) << 7)
        | (u32::from((instruction >> 12) & 1) << 9);
    sign_extend(encoded, 10)
}

pub(super) fn decode_c_jump(instruction: u16) -> i32 {
    let encoded = (u32::from((instruction >> 3) & 0x7) << 1)
        | (u32::from((instruction >> 11) & 1) << 4)
        | (u32::from((instruction >> 2) & 1) << 5)
        | (u32::from((instruction >> 7) & 1) << 6)
        | (u32::from((instruction >> 6) & 1) << 7)
        | (u32::from((instruction >> 9) & 0x3) << 8)
        | (u32::from((instruction >> 8) & 1) << 10)
        | (u32::from((instruction >> 12) & 1) << 11);
    sign_extend(encoded, 12)
}

pub(super) fn decode_c_branch(instruction: u16) -> i32 {
    let encoded = (u32::from((instruction >> 3) & 0x3) << 1)
        | (u32::from((instruction >> 10) & 0x3) << 3)
        | (u32::from((instruction >> 2) & 1) << 5)
        | (u32::from((instruction >> 5) & 0x3) << 6)
        | (u32::from((instruction >> 12) & 1) << 8);
    sign_extend(encoded, 9)
}
