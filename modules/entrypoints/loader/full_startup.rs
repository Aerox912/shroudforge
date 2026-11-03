//! A full-mode failure is terminal. Deliberately no recovery/updater callback.
pub fn run(verify:impl FnOnce()->Result<(),String>,native:impl FnOnce()->Result<(),String>)->Result<(),String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(||{verify()?;native()}))
        .unwrap_or_else(|_|Err("Full startup panicked; no-copy startup stopped".into()))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn full_success_verifies_before_native() {
        let order=std::cell::RefCell::new(Vec::new());
        run(||{order.borrow_mut().push("verify");Ok(())},||{order.borrow_mut().push("native");Ok(())}).unwrap();
        assert_eq!(*order.borrow(),["verify","native"]);
    }
    #[test] fn verification_error_or_panic_never_enters_native_or_recovery() {
        for panic in [false,true] {
            let calls=std::cell::Cell::new(0);
            assert!(run(||{if panic{panic!("synthetic verifier panic")};Err("missing prepared policy".into())},||{calls.set(calls.get()+1);Ok(())}).is_err());
            assert_eq!(calls.get(),0);
        }
    }
    #[test] fn native_error_or_panic_is_terminal() {
        assert!(run(||Ok(()),||Err("required native unavailable".into())).is_err());
        assert!(run(||Ok(()),||panic!("synthetic native panic")).is_err());
    }
}
