use std::fs::File;
use std::io::SeekFrom;
use std::io::{Read, Seek};
use std::path::Path;

#[derive(serde::Serialize)]
pub(crate) struct LogTail {
    pub(crate) text: String,
    pub(crate) state: &'static str,
}

pub(crate) fn read_tail(path: &Path, maximum: u64) -> LogTail {
    let Ok(mut file) = File::open(path) else {
        return LogTail {
            text: String::new(),
            state: if path.exists() { "error" } else { "missing" },
        };
    };
    let Ok(end) = file.seek(SeekFrom::End(0)) else {
        return LogTail {
            text: String::new(),
            state: "error",
        };
    };
    let count = end.min(maximum);
    let start = end - count;
    let mut partial = false;
    if start > 0 {
        let mut previous = [0_u8; 1];
        if file.seek(SeekFrom::Start(start - 1)).is_err() || file.read_exact(&mut previous).is_err()
        {
            return LogTail {
                text: String::new(),
                state: "error",
            };
        }
        partial = previous[0] != b'\n';
    }
    if file.seek(SeekFrom::Start(start)).is_err() {
        return LogTail {
            text: String::new(),
            state: "error",
        };
    }
    let mut bytes = Vec::with_capacity(count as usize);
    if file.take(count).read_to_end(&mut bytes).is_err() {
        return LogTail {
            text: String::new(),
            state: "error",
        };
    }
    if partial {
        if let Some(newline) = bytes.iter().position(|byte| *byte == b'\n') {
            bytes.drain(..=newline);
        } else {
            bytes.clear();
        }
    }
    LogTail {
        text: String::from_utf8_lossy(&bytes).into_owned(),
        state: "ready",
    }
}
