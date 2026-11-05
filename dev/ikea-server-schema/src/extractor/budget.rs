use std::{
    cell::RefCell,
    io::{self, Read, Seek},
};
#[derive(Default)]
struct Budget {
    entries: usize,
    strings: usize,
    defaults: usize,
    unresolved: usize,
    current_type: usize,
    reference_issues: Vec<serde_json::Value>,
    attribute_collisions: Vec<serde_json::Value>,
    attribute_owner: String,
}
thread_local! {static STATE:RefCell<Budget>=RefCell::new(Budget::default());}
pub fn reset() {
    STATE.with(|s| *s.borrow_mut() = Budget::default());
}
pub fn current_type(index: usize) {
    STATE.with(|s| s.borrow_mut().current_type = index)
}
pub fn unresolved(context: &str) {
    STATE.with(|s|{let mut b=s.borrow_mut();b.unresolved+=1;let index=b.current_type;b.reference_issues.push(serde_json::json!({"typeIndex":index,"field":context,"rawReferenceState":"nonzero_unresolved","SDKOptionalValue":"None","qualification":"unsupported_presence_not_known_absence"}));})
}
pub fn reference_issues() -> Vec<serde_json::Value> {
    STATE.with(|s| s.borrow().reference_issues.clone())
}
pub fn attribute_owner(name: &str) {
    STATE.with(|s| s.borrow_mut().attribute_owner = name.into());
}
pub fn attribute_collision(a: &kfc::reflection::Attribute, b: &kfc::reflection::Attribute) {
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        let index = state.current_type;
        let owner = state.attribute_owner.clone();
        state
            .attribute_collisions
            .push(serde_json::json!({"typeIndex":index,"map":owner,"previous":a,"current":b}));
    });
}
pub fn collisions() -> Vec<serde_json::Value> {
    STATE.with(|s| s.borrow().attribute_collisions.clone())
}
pub fn unresolved_count() -> usize {
    STATE.with(|s| s.borrow().unresolved)
}
pub fn err() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "strict reflection bound or metadata contract refused",
    )
}
pub fn bound(n: u64, max: usize) -> io::Result<usize> {
    let n = usize::try_from(n).map_err(|_| err())?;
    if n > max {
        return Err(err());
    }
    Ok(n)
}
pub fn entries(n: u64) -> io::Result<usize> {
    let n = bound(n, 4096)?;
    STATE.with(|s| {
        let mut b = s.borrow_mut();
        b.entries = b.entries.checked_add(n).ok_or_else(err)?;
        if b.entries > 262144 {
            return Err(err());
        }
        Ok(n)
    })
}
pub fn defaults(n: u64) -> io::Result<usize> {
    let n = bound(n, 1048576)?;
    STATE.with(|s| {
        let mut b = s.borrow_mut();
        b.defaults = b.defaults.checked_add(n).ok_or_else(err)?;
        if b.defaults > 33554432 {
            return Err(err());
        }
        Ok(n)
    })
}
pub trait ReadExt: Read + Seek {
    fn read_u8(&mut self) -> io::Result<u8> {
        let mut b = [0];
        self.read_exact(&mut b)?;
        Ok(b[0])
    }
    fn read_u16(&mut self) -> io::Result<u16> {
        let mut b = [0; 2];
        self.read_exact(&mut b)?;
        Ok(u16::from_le_bytes(b))
    }
    fn read_u32(&mut self) -> io::Result<u32> {
        let mut b = [0; 4];
        self.read_exact(&mut b)?;
        Ok(u32::from_le_bytes(b))
    }
    fn read_u64(&mut self) -> io::Result<u64> {
        let mut b = [0; 8];
        self.read_exact(&mut b)?;
        Ok(u64::from_le_bytes(b))
    }
    fn padding(&mut self, n: usize) -> io::Result<()> {
        bound(n as u64, 128)?;
        let mut b = [0; 128];
        self.read_exact(&mut b[..n])
    }
    fn read_string(&mut self, n: usize) -> io::Result<String> {
        bound(n as u64, 65536)?;
        STATE.with(|s| {
            let mut b = s.borrow_mut();
            b.strings = b.strings.checked_add(n).ok_or_else(err)?;
            if b.strings > 33554432 {
                return Err(err());
            }
            Ok(())
        })?;
        let mut b = vec![0; n];
        self.read_exact(&mut b)?;
        String::from_utf8(b).map_err(|_| err())
    }
}
impl<T: Read + Seek> ReadExt for T {}
