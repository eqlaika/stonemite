use std::sync::atomic::{AtomicU32, AtomicU64, AtomicU8, Ordering};

pub const MAGIC: u32 = 0x5354_4D54; // "STMT"
pub const VERSION: u32 = 4;
pub const PROXY_READY: u32 = 0xA5;
pub const HEARTBEAT_TIMEOUT_MS: u32 = 500;
pub const KEY_COUNT: usize = 256;
pub const CONTROLLER_EVENT_CAPACITY: usize = 1024;
pub const KEYBOARD_BROADCAST: u32 = 1 << 0;
pub const KEYBOARD_TARGETED: u32 = 1 << 1;
pub const KEYBOARD_CONTROLLER_MASK: u32 = KEYBOARD_BROADCAST | KEYBOARD_TARGETED;

/// Versioned shared-memory ABI used by Stonemite and the trusik proxy.
///
/// Every field that can change after publication is atomic. Controller-owned
/// and auto-type key levels have separate buffers, sequences, activation, and
/// heartbeats so either owner can fail without preserving or erasing the other.
#[repr(C)]
pub struct SharedKeyState {
    pub magic: AtomicU32,
    pub version: AtomicU32,
    pub proxy_ready: AtomicU32,
    pub suppress: AtomicU32,
    pub controller_keyboard_active: AtomicU32,
    pub controller_mouse_active: AtomicU32,
    pub controller_sequence: AtomicU32,
    pub controller_heartbeat_ms: AtomicU32,
    pub auto_type_generation: AtomicU32,
    reserved_before_lease: u32,
    auto_type_lease: AtomicU64,
    reserved: [u32; 4],
    controller_event_head: AtomicU32,
    reserved_before_events: u32,
    controller_events: [AtomicU64; CONTROLLER_EVENT_CAPACITY],
    controller_keys: [AtomicU8; KEY_COUNT],
    auto_type_keys: [AtomicU64; KEY_COUNT],
}

impl SharedKeyState {
    fn unpublished() -> Self {
        Self {
            magic: AtomicU32::new(0),
            version: AtomicU32::new(0),
            proxy_ready: AtomicU32::new(0),
            suppress: AtomicU32::new(0),
            controller_keyboard_active: AtomicU32::new(0),
            controller_mouse_active: AtomicU32::new(0),
            controller_sequence: AtomicU32::new(0),
            controller_heartbeat_ms: AtomicU32::new(0),
            auto_type_generation: AtomicU32::new(0),
            reserved_before_lease: 0,
            auto_type_lease: AtomicU64::new(0),
            reserved: [0; 4],
            controller_event_head: AtomicU32::new(0),
            reserved_before_events: 0,
            controller_events: [const { AtomicU64::new(0) }; CONTROLLER_EVENT_CAPACITY],
            controller_keys: [const { AtomicU8::new(0) }; KEY_COUNT],
            auto_type_keys: [const { AtomicU64::new(0) }; KEY_COUNT],
        }
    }

    /// Construct and publish a newly-created mapping.
    ///
    /// # Safety
    ///
    /// `ptr` must point to writable, properly aligned storage of at least
    /// `size_of::<SharedKeyState>()` bytes that no reader has accepted yet.
    pub unsafe fn initialize(ptr: *mut Self) {
        ptr.write(Self::unpublished());
        let state = &*ptr;
        state.magic.store(MAGIC, Ordering::Relaxed);
        state.version.store(VERSION, Ordering::Release);
    }

    pub fn is_compatible(&self) -> bool {
        self.version.load(Ordering::Acquire) == VERSION
            && self.magic.load(Ordering::Acquire) == MAGIC
    }

    pub fn acknowledge_proxy(&self) {
        if self.is_compatible() {
            self.proxy_ready.store(PROXY_READY, Ordering::Release);
        }
    }

    pub fn proxy_is_ready(&self) -> bool {
        self.is_compatible() && self.proxy_ready.load(Ordering::Acquire) == PROXY_READY
    }

    pub fn refresh_controller_heartbeat(&self, now_ms: u32) {
        self.controller_heartbeat_ms
            .store(now_ms.max(1), Ordering::Release);
    }

    pub fn controller_is_fresh(&self, now_ms: u32) -> bool {
        heartbeat_is_fresh(self.controller_heartbeat_ms.load(Ordering::Acquire), now_ms)
    }

    pub fn auto_type_is_fresh(&self, now_ms: u32) -> bool {
        let lease = self.auto_type_lease.load(Ordering::Acquire);
        lease_generation(lease) != 0 && heartbeat_is_fresh(lease_heartbeat(lease), now_ms)
    }

    pub fn controller_keyboard_is_active(&self, now_ms: u32) -> bool {
        self.controller_keyboard_active.load(Ordering::Acquire) & KEYBOARD_CONTROLLER_MASK != 0
            && self.controller_is_fresh(now_ms)
    }

    pub fn auto_type_is_active(&self, now_ms: u32) -> bool {
        self.auto_type_is_fresh(now_ms)
    }

    pub fn keyboard_is_active(&self, now_ms: u32) -> bool {
        self.controller_keyboard_is_active(now_ms) || self.auto_type_is_active(now_ms)
    }

    pub fn mouse_is_active(&self, now_ms: u32) -> bool {
        self.controller_mouse_active.load(Ordering::Acquire) != 0
            && self.controller_is_fresh(now_ms)
    }

    pub fn is_active(&self, now_ms: u32) -> bool {
        self.keyboard_is_active(now_ms) || self.mouse_is_active(now_ms)
    }

    pub fn should_suppress(&self, now_ms: u32) -> bool {
        self.suppress.load(Ordering::Acquire) != 0 && self.controller_is_fresh(now_ms)
    }

    pub fn key_is_pressed(&self, scan: u8, now_ms: u32) -> bool {
        let index = scan as usize;
        let lease = self.auto_type_lease.load(Ordering::Acquire);
        let generation = lease_generation(lease);
        (self.controller_keyboard_is_active(now_ms)
            && self.controller_keys[index].load(Ordering::Acquire) & 0x80 != 0)
            || (generation != 0
                && heartbeat_is_fresh(lease_heartbeat(lease), now_ms)
                && auto_key_is_pressed(
                    self.auto_type_keys[index].load(Ordering::Acquire),
                    generation,
                ))
    }

    /// Materialize the OR of all fresh active owner buffers.
    pub fn read_effective_keys(&self, now_ms: u32, output: &mut [u8; KEY_COUNT]) -> bool {
        output.fill(0);
        let controller_active = self.controller_keyboard_is_active(now_ms);
        let auto_lease = self.auto_type_lease.load(Ordering::Acquire);
        let auto_generation = lease_generation(auto_lease);
        let auto_type_active =
            auto_generation != 0 && heartbeat_is_fresh(lease_heartbeat(auto_lease), now_ms);

        if controller_active {
            let mut keys = [0u8; KEY_COUNT];
            if read_consistent(&self.controller_sequence, &self.controller_keys, &mut keys) {
                merge_keys(output, &keys);
            }
        }
        if auto_type_active {
            for (output, key) in output.iter_mut().zip(&self.auto_type_keys) {
                if auto_key_is_pressed(key.load(Ordering::Acquire), auto_generation) {
                    *output |= 0x80;
                }
            }
            // If a newer owner started during the snapshot, do not expose a
            // mixed generation. The next poll will read the new owner.
            if lease_generation(self.auto_type_lease.load(Ordering::Acquire)) != auto_generation {
                output.fill(0);
                if controller_active {
                    let mut keys = [0u8; KEY_COUNT];
                    if read_consistent(&self.controller_sequence, &self.controller_keys, &mut keys)
                    {
                        merge_keys(output, &keys);
                    }
                }
            }
        }

        controller_active || auto_type_active
    }

    pub fn set_controller_key(&self, scan: usize, value: u8) {
        write_controller_key(
            &self.controller_sequence,
            &self.controller_event_head,
            &self.controller_events,
            &self.controller_keys,
            scan,
            value,
        );
    }

    pub fn clear_controller_keys(&self) {
        clear_controller_keys(
            &self.controller_sequence,
            &self.controller_event_head,
            &self.controller_events,
            &self.controller_keys,
        );
    }

    pub fn controller_event_head(&self) -> u32 {
        self.controller_event_head.load(Ordering::Acquire)
    }

    /// Visit every controller transition published after `cursor`. Returns
    /// true if the reader fell more than one ring behind and older events were
    /// necessarily discarded.
    pub fn drain_controller_events(
        &self,
        cursor: &mut Option<u32>,
        mut visit: impl FnMut(u8, bool),
    ) -> bool {
        let head = self.controller_event_head.load(Ordering::Acquire);
        let Some(mut next) = *cursor else {
            *cursor = Some(head);
            return false;
        };
        let mut overflowed = head.wrapping_sub(next) as usize > CONTROLLER_EVENT_CAPACITY;
        if overflowed {
            next = head.wrapping_sub(CONTROLLER_EVENT_CAPACITY as u32);
        }
        while next != head {
            let record = self.controller_events[next as usize % CONTROLLER_EVENT_CAPACITY]
                .load(Ordering::Acquire);
            if (record >> 32) as u32 != next {
                overflowed = true;
            } else {
                visit(record as u8, record & (1 << 8) != 0);
            }
            next = next.wrapping_add(1);
        }
        *cursor = Some(head);
        overflowed
    }

    pub fn read_auto_type_keys(&self, now_ms: u32, output: &mut [u8; KEY_COUNT]) -> bool {
        output.fill(0);
        let lease = self.auto_type_lease.load(Ordering::Acquire);
        let generation = lease_generation(lease);
        if generation == 0 || !heartbeat_is_fresh(lease_heartbeat(lease), now_ms) {
            return false;
        }
        for (output, key) in output.iter_mut().zip(&self.auto_type_keys) {
            if auto_key_is_pressed(key.load(Ordering::Acquire), generation) {
                *output = 0x80;
            }
        }
        if lease_generation(self.auto_type_lease.load(Ordering::Acquire)) != generation {
            output.fill(0);
            return false;
        }
        true
    }

    pub fn set_auto_type_key(&self, generation: u32, scan: usize, value: u8) -> bool {
        if scan >= KEY_COUNT {
            return false;
        }
        let key = &self.auto_type_keys[scan];
        let replacement = encode_auto_key(generation, value != 0);
        let mut current = key.load(Ordering::Acquire);
        loop {
            if lease_generation(self.auto_type_lease.load(Ordering::Acquire)) != generation
                || auto_key_generation(current) > generation
            {
                return false;
            }
            match key.compare_exchange_weak(
                current,
                replacement,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return lease_generation(self.auto_type_lease.load(Ordering::Acquire))
                        == generation;
                }
                Err(observed) => current = observed,
            }
        }
    }

    pub fn refresh_auto_type_lease(&self, generation: u32, now_ms: u32) -> bool {
        let mut lease = self.auto_type_lease.load(Ordering::Acquire);
        loop {
            if lease_generation(lease) != generation {
                return false;
            }
            match self.auto_type_lease.compare_exchange_weak(
                lease,
                encode_lease(generation, now_ms),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(current) => lease = current,
            }
        }
    }

    pub fn retire_controller(&self) {
        self.controller_keyboard_active.store(0, Ordering::Release);
        self.controller_mouse_active.store(0, Ordering::Release);
        self.suppress.store(0, Ordering::Release);
        self.controller_heartbeat_ms.store(0, Ordering::Release);
        reset_keys(&self.controller_sequence, &self.controller_keys);
    }

    pub fn begin_auto_type(&self, now_ms: u32) -> u32 {
        let generation = self
            .auto_type_generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        assert_ne!(generation, 0, "auto-type generation exhausted");
        let new_lease = encode_lease(generation, now_ms);
        let mut current = self.auto_type_lease.load(Ordering::Acquire);
        loop {
            if lease_generation(current) >= generation {
                return generation;
            }
            match self.auto_type_lease.compare_exchange_weak(
                current,
                new_lease,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return generation,
                Err(observed) => current = observed,
            }
        }
    }

    pub fn retire_auto_type(&self, generation: u32) {
        let mut lease = self.auto_type_lease.load(Ordering::Acquire);
        loop {
            if lease_generation(lease) != generation {
                return;
            }
            match self.auto_type_lease.compare_exchange_weak(
                lease,
                0,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return,
                Err(current) => lease = current,
            }
        }
    }
}

pub const SHARED_KEY_STATE_SIZE: usize = std::mem::size_of::<SharedKeyState>();

fn encode_lease(generation: u32, heartbeat_ms: u32) -> u64 {
    ((generation as u64) << 32) | heartbeat_ms.max(1) as u64
}

fn lease_generation(lease: u64) -> u32 {
    (lease >> 32) as u32
}

fn lease_heartbeat(lease: u64) -> u32 {
    lease as u32
}

fn encode_auto_key(generation: u32, pressed: bool) -> u64 {
    ((generation as u64) << 1) | u64::from(pressed)
}

fn auto_key_generation(key: u64) -> u32 {
    (key >> 1) as u32
}

fn auto_key_is_pressed(key: u64, generation: u32) -> bool {
    auto_key_generation(key) == generation && key & 1 != 0
}

pub fn heartbeat_is_fresh(heartbeat_ms: u32, now_ms: u32) -> bool {
    heartbeat_ms != 0 && now_ms.wrapping_sub(heartbeat_ms) <= HEARTBEAT_TIMEOUT_MS
}

fn lock_sequence(sequence: &AtomicU32) {
    loop {
        let current = sequence.load(Ordering::Acquire);
        if current & 1 != 0 {
            std::hint::spin_loop();
            continue;
        }
        if sequence
            .compare_exchange_weak(
                current,
                current.wrapping_add(1),
                Ordering::Acquire,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            return;
        }
    }
}

fn unlock_sequence(sequence: &AtomicU32) {
    sequence.fetch_add(1, Ordering::Release);
}

fn publish_controller_event(
    head: &AtomicU32,
    events: &[AtomicU64; CONTROLLER_EVENT_CAPACITY],
    scan: usize,
    pressed: bool,
) {
    let sequence = head.load(Ordering::Relaxed);
    let record = ((sequence as u64) << 32) | scan as u64 | (u64::from(pressed) << 8);
    events[sequence as usize % CONTROLLER_EVENT_CAPACITY].store(record, Ordering::Release);
    head.store(sequence.wrapping_add(1), Ordering::Release);
}

fn write_controller_key(
    sequence: &AtomicU32,
    event_head: &AtomicU32,
    events: &[AtomicU64; CONTROLLER_EVENT_CAPACITY],
    keys: &[AtomicU8; KEY_COUNT],
    scan: usize,
    value: u8,
) {
    if scan >= KEY_COUNT {
        return;
    }
    lock_sequence(sequence);
    let pressed = value & 0x80 != 0;
    if keys[scan].load(Ordering::Relaxed) & 0x80 != value & 0x80 {
        keys[scan].store(if pressed { 0x80 } else { 0 }, Ordering::Relaxed);
        publish_controller_event(event_head, events, scan, pressed);
    }
    unlock_sequence(sequence);
}

fn clear_controller_keys(
    sequence: &AtomicU32,
    event_head: &AtomicU32,
    events: &[AtomicU64; CONTROLLER_EVENT_CAPACITY],
    keys: &[AtomicU8; KEY_COUNT],
) {
    lock_sequence(sequence);
    for (scan, key) in keys.iter().enumerate() {
        if key.load(Ordering::Relaxed) & 0x80 != 0 {
            key.store(0, Ordering::Relaxed);
            publish_controller_event(event_head, events, scan, false);
        }
    }
    unlock_sequence(sequence);
}

/// Reset a buffer while its owner is inactive. This deliberately recovers a
/// sequence left odd by an owner process that terminated during a write.
fn reset_keys(sequence: &AtomicU32, keys: &[AtomicU8; KEY_COUNT]) {
    sequence.store(1, Ordering::Release);
    for key in keys {
        key.store(0, Ordering::Relaxed);
    }
    sequence.store(2, Ordering::Release);
}

fn read_consistent(
    sequence: &AtomicU32,
    keys: &[AtomicU8; KEY_COUNT],
    output: &mut [u8; KEY_COUNT],
) -> bool {
    for _ in 0..8 {
        let before = sequence.load(Ordering::Acquire);
        if before & 1 != 0 {
            std::hint::spin_loop();
            continue;
        }
        for (output, key) in output.iter_mut().zip(keys) {
            *output = key.load(Ordering::Relaxed);
        }
        let after = sequence.load(Ordering::Acquire);
        if before == after && after & 1 == 0 {
            return true;
        }
    }
    // Atomic bytes remain race-free even if a writer was preempted while the
    // sequence was odd. Prefer a possibly mixed snapshot over a fabricated
    // all-released state; owner heartbeats still bound a crashed writer.
    for (output, key) in output.iter_mut().zip(keys) {
        *output = key.load(Ordering::Acquire);
    }
    true
}

fn merge_keys(output: &mut [u8; KEY_COUNT], keys: &[u8; KEY_COUNT]) {
    for (output, key) in output.iter_mut().zip(keys) {
        *output |= *key & 0x80;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> SharedKeyState {
        let mut state = std::mem::MaybeUninit::<SharedKeyState>::uninit();
        unsafe {
            SharedKeyState::initialize(state.as_mut_ptr());
            state.assume_init()
        }
    }

    #[test]
    fn version_four_layout_is_stable() {
        assert_eq!(VERSION, 4);
        assert_eq!(SHARED_KEY_STATE_SIZE, 10_568);
        assert_eq!(std::mem::align_of::<SharedKeyState>(), 8);
        assert_eq!(std::mem::offset_of!(SharedKeyState, magic), 0);
        assert_eq!(std::mem::offset_of!(SharedKeyState, version), 4);
        assert_eq!(std::mem::offset_of!(SharedKeyState, proxy_ready), 8);
        assert_eq!(std::mem::offset_of!(SharedKeyState, suppress), 12);
        assert_eq!(
            std::mem::offset_of!(SharedKeyState, controller_keyboard_active),
            16
        );
        assert_eq!(
            std::mem::offset_of!(SharedKeyState, controller_mouse_active),
            20
        );
        assert_eq!(
            std::mem::offset_of!(SharedKeyState, auto_type_generation),
            32
        );
        assert_eq!(std::mem::offset_of!(SharedKeyState, auto_type_lease), 40);
        assert_eq!(
            std::mem::offset_of!(SharedKeyState, controller_event_head),
            64
        );
        assert_eq!(std::mem::offset_of!(SharedKeyState, controller_events), 72);
        assert_eq!(std::mem::offset_of!(SharedKeyState, controller_keys), 8_264);
        assert_eq!(std::mem::offset_of!(SharedKeyState, auto_type_keys), 8_520);
    }

    #[test]
    fn controller_ring_preserves_repeated_key_edges_between_reads() {
        let state = state();
        let mut cursor = Some(0);
        state.set_controller_key(0x26, 0x80);
        state.set_controller_key(0x26, 0);
        state.set_controller_key(0x26, 0x80);
        state.set_controller_key(0x26, 0);

        let mut events = Vec::new();
        assert!(
            !state.drain_controller_events(&mut cursor, |scan, pressed| {
                events.push((scan, pressed));
            })
        );
        assert_eq!(
            events,
            vec![(0x26, true), (0x26, false), (0x26, true), (0x26, false)]
        );
    }

    #[test]
    fn owners_compose_without_releasing_each_other() {
        let state = state();
        state.refresh_controller_heartbeat(100);
        state
            .controller_keyboard_active
            .store(KEYBOARD_BROADCAST, Ordering::Release);
        state.set_controller_key(0x1e, 0x80);
        let auto_generation = state.begin_auto_type(100);
        state.set_auto_type_key(auto_generation, 0x1e, 0x80);

        state.retire_controller();
        let mut keys = [0; KEY_COUNT];
        assert!(state.read_effective_keys(100, &mut keys));
        assert_eq!(keys[0x1e], 0x80);

        state.retire_auto_type(auto_generation);
        assert!(!state.read_effective_keys(100, &mut keys));
        assert_eq!(keys[0x1e], 0);
    }

    #[test]
    fn owner_heartbeats_expire_independently_after_a_crash() {
        let state = state();
        state.refresh_controller_heartbeat(100);
        state
            .controller_keyboard_active
            .store(KEYBOARD_TARGETED, Ordering::Release);
        state.controller_mouse_active.store(1, Ordering::Release);
        state.suppress.store(1, Ordering::Release);
        state.set_controller_key(0x1e, 0x80);
        let auto_generation = state.begin_auto_type(400);
        state.set_auto_type_key(auto_generation, 0x30, 0x80);

        let mut keys = [0; KEY_COUNT];
        assert!(state.read_effective_keys(601, &mut keys));
        assert_eq!(keys[0x1e], 0);
        assert_eq!(keys[0x30], 0x80);
        assert!(!state.mouse_is_active(601));
        assert!(!state.should_suppress(601));

        state.refresh_controller_heartbeat(1_000);
        state.retire_auto_type(auto_generation);
        state.set_controller_key(0x1e, 0x80);
        assert!(state.read_effective_keys(1_000, &mut keys));
        assert_eq!(keys[0x1e], 0x80);
        assert_eq!(keys[0x30], 0);
    }

    #[test]
    fn concurrent_auto_type_starts_leave_only_the_latest_generation_active() {
        use std::sync::{Arc, Barrier};

        let state = Arc::new(state());
        let barrier = Arc::new(Barrier::new(16));
        let threads: Vec<_> = (0..16)
            .map(|_| {
                let state = Arc::clone(&state);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    let generation = state.begin_auto_type(100);
                    let accepted = state.set_auto_type_key(generation, 0x1e, 0x80);
                    (generation, accepted)
                })
            })
            .collect();
        let results: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        let latest = results
            .iter()
            .map(|(generation, _)| *generation)
            .max()
            .unwrap();
        assert_eq!(
            lease_generation(state.auto_type_lease.load(Ordering::Acquire)),
            latest
        );
        assert!(
            results
                .iter()
                .filter(|(generation, accepted)| *generation == latest && *accepted)
                .count()
                == 1
        );
    }

    #[test]
    fn stale_auto_type_writer_cannot_erase_a_newer_key() {
        let state = state();
        let stale_generation = state.begin_auto_type(100);
        let stale_observation = state.auto_type_keys[0x1e].load(Ordering::Acquire);

        let current_generation = state.begin_auto_type(101);
        assert!(state.set_auto_type_key(current_generation, 0x1e, 0x80));
        assert!(state.auto_type_keys[0x1e]
            .compare_exchange(
                stale_observation,
                encode_auto_key(stale_generation, false),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err());
        assert!(!state.set_auto_type_key(stale_generation, 0x1e, 0));

        let mut keys = [0; KEY_COUNT];
        assert!(state.read_effective_keys(101, &mut keys));
        assert_eq!(keys[0x1e], 0x80);
    }

    #[test]
    fn heartbeat_age_handles_tick_count_wrap() {
        assert!(heartbeat_is_fresh(u32::MAX - 20, 10));
        assert!(!heartbeat_is_fresh(
            u32::MAX - HEARTBEAT_TIMEOUT_MS,
            HEARTBEAT_TIMEOUT_MS
        ));
    }
}
