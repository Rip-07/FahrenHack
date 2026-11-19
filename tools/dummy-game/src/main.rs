//! Test "game" for FahrenHack end-to-end checks.
//! HP lives behind a real pointer chain: static WORLD -> World (+0x40) -> Player (+0x18) -> hp.
//! HP drops by 7 every second, ammo by 1 every 3 seconds; both are printed each tick.

use std::hint::black_box;
use std::sync::atomic::{AtomicI32, AtomicPtr, Ordering};

#[repr(C)]
struct Player {
    pad: [u64; 3],
    hp: AtomicI32,
    ammo: AtomicI32,
}

#[repr(C)]
struct World {
    pad: [u8; 0x40],
    player: Box<Player>,
}

static WORLD: AtomicPtr<World> = AtomicPtr::new(std::ptr::null_mut());

fn main() {
    let w = Box::new(World {
        pad: [0; 0x40],
        player: Box::new(Player { pad: [0; 3], hp: AtomicI32::new(1000), ammo: AtomicI32::new(30) }),
    });
    WORLD.store(Box::into_raw(w), Ordering::SeqCst);
    let mut tick = 0u64;
    loop {
        let w = unsafe { &*black_box(WORLD.load(Ordering::SeqCst)) };
        println!("hp={} ammo={}", w.player.hp.load(Ordering::SeqCst), w.player.ammo.load(Ordering::SeqCst));
        tick += 1;
        w.player.hp.fetch_sub(7, Ordering::SeqCst);
        if tick % 3 == 0 {
            let a = w.player.ammo.load(Ordering::SeqCst);
            w.player.ammo.store(if a <= 1 { 30 } else { a - 1 }, Ordering::SeqCst);
        }
        std::thread::sleep(std::time::Duration::from_millis(1000));
    }
}
