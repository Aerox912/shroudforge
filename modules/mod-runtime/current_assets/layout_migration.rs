//! Exact HD Map 0.2.0 -> 0.2.1 descriptor migration. No broad same-schema exception.
use super::*;
const GPC: &str = "7df551f7-ddd1-52b0-8ba6-d09be35dbccc";
const OLD: &str = "bf43423e8f39870cce1cdbbdfcbef76337367f889c93bf782019155d1fb5a51b";
const NEW: &str = "72c086eecc5a1852c50367109c0126f86102a1e472b7c07546fa4885c078ebe8";
const PACKAGE: &str = "90931db91a9a3bcd372d627422aafdaedcb3e7a83f2ea4dfd15d24ade6f3512f";

#[derive(Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub(super) struct Contract {
    pub state_sha256: String,
    pub fingerprint: String,
    pub policy: String,
    pub policy_sha256: String,
    #[serde(default)]
    pub loader_replacement: Option<LoaderReplacement>,
}
#[derive(Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub(super) struct LoaderReplacement {
    name: String,
    before_sha256: String,
    after_sha256: String,
}
fn digest(value:&str)->bool { value.len()==64 && value.bytes().all(|b|b.is_ascii_hexdigit()) }

pub(super) fn predecessor(root:&Path, policy:&Policy)->Result<Option<Policy>> {
    let Some(c)=&policy.layout_migration else { return Ok(None); };
    ensure!(policy.schema_version==2 && digest(&c.state_sha256) && digest(&c.fingerprint) && digest(&c.policy_sha256),"invalid layout migration identity");
    let bytes=original(root,&c.policy,&c.policy_sha256)?;
    let old:Policy=serde_json::from_slice(&bytes)?;
    ensure!(old.schema_version==2 && old.layout_migration.is_none(),"unsupported layout predecessor");
    let mut expected:serde_json::Value=serde_json::from_slice(&bytes)?;
    let mut actual:serde_json::Value=read_json(&joined(root,POLICY)?)?;
    actual.as_object_mut().context("layout policy object")?.remove("layoutMigration");
    let records=expected["resources"].as_array_mut().context("predecessor records")?;
    let gpc=records.iter_mut().find(|r|r["id"]==GPC).context("predecessor GPC")?;
    ensure!(gpc["typeHash"]==0x647628d6u32 && gpc["appliedSha256"]==OLD,"unsupported predecessor GPC");
    gpc["appliedSha256"]=NEW.into();
    if let Some(loader)=&c.loader_replacement {
        ensure!(["winmm.dll","shroudtopia.dll","dbgcore.dll","dinput8.dll"].contains(&loader.name.as_str()) &&
            digest(&loader.before_sha256) && digest(&loader.after_sha256) && loader.before_sha256!=loader.after_sha256,
            "invalid reviewed loader replacement");
        ensure!(expected["preservedLoaders"][&loader.name]==loader.before_sha256,"predecessor loader pin mismatch");
        expected["preservedLoaders"][&loader.name]=loader.after_sha256.clone().into();
    }
    ensure!(expected==actual,"layout successor changed unrelated recipe identity");
    Ok(Some(old))
}
pub(super) fn package(inputs:&BTreeMap<String,String>)->Result<()> {
    ensure!(hash(&serde_json::to_vec(inputs)?)==PACKAGE,"layout migration requires exact reviewed 0.2.1 package"); Ok(())
}
pub(super) fn authorize(root:&Path,state:&Prepared,c:&Contract)->Result<()> {
    ensure!(state.schema_version==2 && state.fingerprint==c.fingerprint && stamp(&joined(root,STATE)?)?.1==c.state_sha256,
        "layout predecessor state/fingerprint mismatch"); Ok(())
}
pub(super) fn before_recovery(root:&Path,fingerprint:&str,c:&Contract)->Result<()> {
    let state:Prepared=read_json(&joined(root,STATE)?)?;validate_shape(root,&state)?;
    if state.fingerprint!=fingerprint { authorize(root,&state,c)?; }
    Ok(())
}
pub(super) fn gpc_id()->ResourceId { ResourceId::parse(GPC,0x647628d6,0).unwrap() }
pub(super) fn corrected(bytes:&[u8])->Result<Vec<u8>> {
    ensure!(hash(bytes)==OLD && bytes.get(548)==Some(&1),"layout predecessor bytes mismatch");
    let mut next=bytes.to_vec();next[548]=2;
    ensure!(hash(&next)==NEW,"layout corrected bytes mismatch");Ok(next)
}
