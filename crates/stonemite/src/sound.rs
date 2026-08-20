use windows::core::PCSTR;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Media::Audio::{PlaySoundA, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};

pub const DEFAULT_SOUND_ID: &str = "tell.wav";

pub struct BuiltinSound {
    pub id: &'static str,
    pub label: &'static str,
    bytes: &'static [u8],
}

pub static BUILTIN_SOUNDS: &[BuiltinSound] = &[
    BuiltinSound {
        id: "alert.wav",
        label: "Alert",
        bytes: include_bytes!("../assets/audio-triggers/alert.wav"),
    },
    BuiltinSound {
        id: "alert2.wav",
        label: "Alert 2",
        bytes: include_bytes!("../assets/audio-triggers/alert2.wav"),
    },
    BuiltinSound {
        id: "alert3.wav",
        label: "Alert 3",
        bytes: include_bytes!("../assets/audio-triggers/alert3.wav"),
    },
    BuiltinSound {
        id: "alert4.wav",
        label: "Alert 4",
        bytes: include_bytes!("../assets/audio-triggers/alert4.wav"),
    },
    BuiltinSound {
        id: "alert5.wav",
        label: "Alert 5",
        bytes: include_bytes!("../assets/audio-triggers/alert5.wav"),
    },
    BuiltinSound {
        id: "tell.wav",
        label: "Tell",
        bytes: include_bytes!("../assets/audio-triggers/tell.wav"),
    },
    BuiltinSound {
        id: "tell2.wav",
        label: "Tell 2",
        bytes: include_bytes!("../assets/audio-triggers/tell2.wav"),
    },
    BuiltinSound {
        id: "thump.wav",
        label: "Thump",
        bytes: include_bytes!("../assets/audio-triggers/thump.wav"),
    },
    BuiltinSound {
        id: "thump2.wav",
        label: "Thump 2",
        bytes: include_bytes!("../assets/audio-triggers/thump2.wav"),
    },
];

pub fn normalized_id(id: &str) -> &'static str {
    find(id).map_or(DEFAULT_SOUND_ID, |sound| sound.id)
}

pub fn label(id: &str) -> &'static str {
    find(id)
        .or_else(|| find(DEFAULT_SOUND_ID))
        .map_or("Tell", |sound| sound.label)
}

pub fn play(id: &str) -> bool {
    let Some(sound) = find(normalized_id(id)) else {
        return false;
    };
    // SND_MEMORY treats the PCSTR as a pointer to a complete WAV image. The
    // include_bytes storage is static, so it remains alive for async playback.
    unsafe {
        PlaySoundA(
            PCSTR(sound.bytes.as_ptr()),
            HMODULE::default(),
            SND_ASYNC | SND_MEMORY | SND_NODEFAULT,
        )
        .as_bool()
    }
}

fn find(id: &str) -> Option<&'static BuiltinSound> {
    BUILTIN_SOUNDS
        .iter()
        .find(|sound| sound.id.eq_ignore_ascii_case(id))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn bundled_sound_ids_are_unique_valid_wave_files() {
        let mut ids = HashSet::new();
        for sound in BUILTIN_SOUNDS {
            assert!(ids.insert(sound.id));
            assert!(sound.bytes.starts_with(b"RIFF"));
            assert_eq!(&sound.bytes[8..12], b"WAVE");
        }
        assert_eq!(BUILTIN_SOUNDS.len(), 9);
    }

    #[test]
    fn invalid_sound_selection_falls_back_to_tell() {
        assert_eq!(normalized_id("not-bundled.wav"), DEFAULT_SOUND_ID);
        assert_eq!(normalized_id("TELL2.WAV"), "tell2.wav");
        assert_eq!(label("not-bundled.wav"), "Tell");
    }

    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "plays an audible bundled sound on the native verification host"]
    fn native_playback_accepts_embedded_wave_memory() {
        assert!(play(DEFAULT_SOUND_ID));
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}
