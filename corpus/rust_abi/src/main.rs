#![no_main]
#![no_std]

use core::arch::global_asm;
use core::panic::PanicInfo;

#[cfg(renvo_wch)]
global_asm!(include_str!("../start-wch.S"));
#[cfg(renvo_esp32c6)]
global_asm!(include_str!("../start-esp32c6.S"));
#[cfg(renvo_rp2040)]
global_asm!(include_str!("../start-arm.S"));
#[cfg(renvo_rp2350_arm)]
global_asm!(include_str!("../start-arm.S"));
#[cfg(renvo_rp2350_riscv)]
global_asm!(include_str!("../start-rp2350-riscv.S"));

#[derive(Clone, Copy)]
#[repr(C)]
struct Pair {
    left: u32,
    right: u32,
}

static INPUT: [u32; 8] = [
    0x1357_9bdf,
    0x2468_ace0,
    0xffff_ffff,
    0x8000_0001,
    17,
    31,
    0x55aa_55aa,
    0xaa55_aa55,
];

#[inline(never)]
fn rotate_mix(pair: Pair, amount: u32) -> Pair {
    Pair {
        left: pair.left.rotate_left(amount & 31) ^ pair.right,
        right: pair.right.rotate_right((amount + 7) & 31).wrapping_add(pair.left),
    }
}

#[inline(never)]
fn fold(values: &[u32]) -> Pair {
    let mut state = Pair {
        left: 0x811c_9dc5,
        right: 0x9e37_79b9,
    };
    let mut index = 0;
    while index < values.len() {
        state.left = state.left.wrapping_add(values[index] ^ index as u32);
        state.right ^= values[index].wrapping_sub(state.left);
        state = rotate_mix(state, (index as u32).wrapping_mul(5).wrapping_add(3));
        index += 1;
    }
    state
}

#[inline(never)]
extern "C" fn check_abi(a: u32, b: u32, c: u32, d: u32, pair: Pair) -> u32 {
    a.wrapping_add(b.rotate_left(3))
        ^ c.wrapping_sub(d)
        ^ pair.left
        ^ pair.right.rotate_right(9)
}

#[no_mangle]
pub extern "C" fn rust_main() -> u32 {
    let state = fold(&INPUT);
    let observed = check_abi(3, 5, 7, 11, state);
    const EXPECTED: u32 = 0x6cab_32d4;
    u32::from(observed != EXPECTED)
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
