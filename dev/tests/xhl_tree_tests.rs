#[path="../../modules/mod-runtime/current_assets/xhl_tree.rs"] mod tree;
use std::{fs,path::PathBuf,process::{Command,Stdio}};
fn base()->PathBuf {
    let parent=PathBuf::from(std::env::var_os("SF_XHL_FIXTURE_ROOT").expect("explicit E fixture root required"));
    assert!(parent.is_absolute()&&parent.to_string_lossy().starts_with("E:\\Scratch\\sf177-xhl-chain-"));
    parent.join(format!("tree-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()))
}
#[test] fn owned_child(){if std::env::var_os("SF_XHL_CHILD").is_some(){std::thread::sleep(std::time::Duration::from_millis(900));}}
#[test] fn exact_membership_and_process_lifetime_seal(){
    let root=base();let fresh=tree::FreshTree::create(&root,&["one".into(),"one/nested".into()]).unwrap();
    fresh.write_new("one/plugin.dll",b"inert fixture, never loaded").unwrap();
    let sealed=fresh.seal().unwrap();let files=vec!["one/plugin.dll".to_string()];
    let lease=tree::TreeLease::validate(&root,&sealed.identities,&files).unwrap();
    let container=tree::TreeLease::pin_container(&root,&sealed.identities[0]).unwrap();
    assert!(tree::FreshTree::create(&root,&[]).is_err());
    for dir in [root.clone(),root.join("one"),root.join("one/nested")] {
        assert_eq!(fs::write(dir.join("injected.dll"),b"not permitted").unwrap_err().kind(),std::io::ErrorKind::PermissionDenied);
        assert_eq!(fs::create_dir(dir.join("injected")).unwrap_err().kind(),std::io::ErrorKind::PermissionDenied);
        assert!(fs::rename(&dir,dir.with_extension("moved")).is_err());
    }
    assert!(fs::write(root.join("one/plugin.dll"),b"overwrite").is_err());
    assert!(fs::remove_file(root.join("one/plugin.dll")).is_err());
    assert!(tree::TreeLease::validate(&root,&sealed.identities,&[]).is_err());
    let mut changed=sealed.identities.clone();changed[0].id[0]^=1;
    assert!(tree::TreeLease::validate(&root,&changed,&files).is_err());
    let outside=root.with_extension("config.json");fs::write(&outside,b"writable outside code tree").unwrap();
    let child=Command::new(std::env::current_exe().unwrap()).args(["--exact","owned_child","--nocapture"]).env("SF_XHL_CHILD","1").stdout(Stdio::null()).spawn().unwrap();
    let mut running=sealed.bind_process(child);assert!(running.restore_when_exited().is_err());
    std::thread::sleep(std::time::Duration::from_millis(1400));drop(lease);drop(container);
    running.restore_when_exited().unwrap();assert!(running.restore_when_exited().is_err());
    fs::write(root.join("one/nested/maintenance.txt"),b"restored after exact child exit").unwrap();
    // Exact task-owned fixture root verified above; preserve source/evidence.
    fs::remove_dir_all(&root).unwrap();fs::remove_file(outside).unwrap();
}
#[test] fn existing_tree_is_never_modified_and_unlaunched_restore(){
    let root=base();fs::create_dir(&root).unwrap();fs::write(root.join("sentinel"),b"unchanged").unwrap();
    assert!(tree::FreshTree::create(&root,&[]).is_err());assert_eq!(fs::read(root.join("sentinel")).unwrap(),b"unchanged");fs::remove_dir_all(&root).unwrap();
    let root=base();let seal=tree::FreshTree::create(&root,&[]).unwrap().seal().unwrap();seal.restore_unlaunched().unwrap();fs::remove_dir(&root).unwrap();
}
#[test] fn core_parent_chain_input_stays_pinned_and_log_writable(){
    use std::os::windows::fs::OpenOptionsExt;
    let root=base();let container=tree::FreshTree::create(&root,&[]).unwrap();
    container.write_disabled_chain_new().unwrap();assert!(container.write_disabled_chain_new().is_err());
    let chain=root.join("dinput8-chain.ini");assert_eq!(fs::read(&chain).unwrap(),tree::DISABLED_CHAIN);
    let _held=fs::OpenOptions::new().read(true).share_mode(1).open(&chain).unwrap();
    assert!(fs::write(&chain,b"[Options]\r\nEnabled=1\r\n").is_err());
    assert!(fs::rename(&chain,root.join("replaced.ini")).is_err());
    fs::write(root.join("XHL-Native-Plugin-Host.log"),b"writable outside scan seal").unwrap();
    let pinned=tree::TreeLease::pin_container(&root,&container.identity().unwrap()).unwrap();
    drop(pinned);drop(_held);drop(container);fs::remove_dir_all(&root).unwrap();
}
fn completed_child()->std::process::Child {
    let mut child=Command::new(std::env::current_exe().unwrap()).args(["--exact","owned_child"]).stdout(Stdio::null()).spawn().unwrap();
    assert!(child.wait().unwrap().success());child
}
#[test] fn running_mid_restore_failure_preserves_progress_and_refuses_identity_or_acl_drift(){
    use tree::{DirectoryProgress as P,test_faults as faults};faults::reset(&[],&[]);
    let root=base();let sealed=tree::FreshTree::create(&root,&["one".into(),"one/nested".into()]).unwrap().seal().unwrap();
    let mut running=sealed.bind_process(completed_child());faults::reset(&[2],&[]);
    assert!(running.restore_when_exited().is_err());
    assert_eq!(running.recovery_progress().unwrap(),[P::Restored,P::RestorePending,P::Sealed]);
    assert!(fs::rename(&root,root.with_extension("moved")).is_err()); // directory leases survived the error
    faults::clear_failures();faults::set_identity_fault(true);
    assert!(running.restore_when_exited().unwrap_err().to_string().contains("identity drift"));assert_eq!(faults::calls(),2);
    faults::set_identity_fault(false);running.test_drift_acl(0,false).unwrap();
    assert!(running.restore_when_exited().unwrap_err().to_string().contains("DACL drift"));assert_eq!(faults::calls(),2);
    // Restore the fixture's deliberate external change, using its saved original.
    // The production retry itself must never overwrite an unexpected DACL.
    running.test_drift_acl(0,true).unwrap();running.restore_when_exited().unwrap();
    assert_eq!(faults::calls(),4); // root was already verified and was not rewritten
    assert!(running.recovery_progress().is_none());fs::remove_dir_all(root).unwrap();faults::reset(&[],&[]);
}
#[test] fn unlaunched_after_write_error_reconciles_the_original_attempt_without_rewriting(){
    use tree::{DirectoryProgress as P,test_faults as faults};faults::reset(&[],&[]);
    let root=base();let sealed=tree::FreshTree::create(&root,&["one".into(),"one/nested".into()]).unwrap().seal().unwrap();
    faults::reset(&[],&[2]);let mut failure=sealed.restore_unlaunched().unwrap_err();
    assert_eq!(failure.progress(),[P::Restored,P::RestorePending,P::Sealed]);assert!(!failure.restoration_complete());
    assert!(fs::rename(&root,root.with_extension("moved")).is_err());
    faults::clear_failures();failure.resume_restore().unwrap();assert!(failure.restoration_complete());
    assert_eq!(faults::calls(),3); // failed call2 had actually applied, so only the final child needed a write
    drop(failure);fs::remove_dir_all(root).unwrap();faults::reset(&[],&[]);
}
#[test] fn mid_seal_and_rollback_errors_return_recoverable_ownership(){
    use tree::{DirectoryProgress as P,test_faults as faults};
    for rollback_fails in [false,true] {
        faults::reset(&[],&[]);let root=base();let fresh=tree::FreshTree::create(&root,&["one".into(),"one/nested".into()]).unwrap();
        faults::reset(if rollback_fails{&[2,3]}else{&[2]},&[]);
        let mut failure=match fresh.seal(){Ok(_)=>panic!("injected seal must fail"),Err(failure)=>failure};
        assert!(failure.source.to_string().contains("before write"));assert_eq!(failure.rollback_error.is_some(),rollback_fails);
        assert_eq!(failure.restoration_complete(),!rollback_fails);
        if rollback_fails {
            assert_eq!(failure.progress(),[P::Restored,P::Restored,P::RestorePending]);
            assert!(fs::rename(&root,root.with_extension("moved")).is_err());
        }
        faults::clear_failures();failure.resume_restore().unwrap();assert!(failure.restoration_complete());
        assert!(failure.progress().iter().all(|p|*p==P::Restored));
        assert_eq!(faults::calls(),if rollback_fails{4}else{3});
        drop(failure);fs::remove_dir_all(root).unwrap();
    }faults::reset(&[],&[]);
}
