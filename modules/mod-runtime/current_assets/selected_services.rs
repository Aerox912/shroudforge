//! Exact selected startup inputs; this grants no World, row or action authority.
use super::*;

pub const METADATA:u32=1;
pub const GENERIC_HOOKS:u32=2;
pub const MAP_CAPTURE:u32=4;
pub const FIRST_PERSON:u32=8;
pub const GRAPHICS:u32=16;
pub const SHARED_PROVIDER:u32=32;
pub const FLIGHT_SCALAR:u32=64;
pub const VEIN:u32=128;
pub const ALL_SERVICES:u32=255;
pub const DEDICATED_GUEST:u32=1;
pub const CURSOR_GATE:u32=2;
pub const FLIGHT_TRANSPORT:u32=4;
pub const VEIN_INI:&str="mods/shroudweave-vein-mining-test.ini";
const FLIGHT_ID:&str="sf-unlimited-flight";
const NAMES:[(&str,u32);8]=[
    ("metadata-init-v1",METADATA),("required-engine-hooks-v1",GENERIC_HOOKS),
    ("map-capture-preparation-v1",MAP_CAPTURE),("first-person-preparation-v1",FIRST_PERSON),
    ("graphics-boot-v1",GRAPHICS),("vein-flight-provider-v1",SHARED_PROVIDER),
    ("flight-scalar-preparation-v2",FLIGHT_SCALAR),("vein-activation-v1",VEIN),
];

#[repr(C)]
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct SelectedBootPlanV2 {
    pub struct_bytes:u32,pub version:u32,pub services:u32,
    pub process_role:u32,pub opt_ins:u32,pub reserved:u32,
}
impl SelectedBootPlanV2 {
    pub fn valid(&self)->bool {
        self.struct_bytes==24&&self.version==2&&self.reserved==0&&self.services&!ALL_SERVICES==0&&
        self.services&METADATA!=0&&self.opt_ins&!7==0&&matches!(self.process_role,1|2)&&
        (self.services&FIRST_PERSON==0||self.services&MAP_CAPTURE!=0)&&
        ((self.services&(FLIGHT_SCALAR|VEIN)!=0)==(self.services&SHARED_PROVIDER!=0))&&
        (self.process_role==1||self.services&(MAP_CAPTURE|FIRST_PERSON|GRAPHICS|SHARED_PROVIDER)==0)&&
        (self.services&SHARED_PROVIDER==0||self.opt_ins&DEDICATED_GUEST!=0)&&
        (self.services&FLIGHT_SCALAR==0||self.opt_ins&FLIGHT_TRANSPORT!=0)&&
        (self.services&VEIN==0||self.opt_ins&CURSOR_GATE!=0)
    }
    pub fn prepared_services(&self)->u32 {self.services&!VEIN}
}
#[derive(Clone,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct SelectedDemand {
    #[serde(default)] pub(crate) xhl:Option<super::xhl_selection::Selection>,
    pub process_role:String,
    #[serde(default)] pub dedicated_guest_qualified:bool,
    #[serde(default)] pub allow_unverified_cursor_gate:bool,
    #[serde(default)] pub flight_transport_enabled:bool,
    #[serde(default)] pub vein_selected:bool,
    pub vein_settings_path:Option<String>,
}
impl Default for SelectedDemand {
    fn default()->Self {Self{xhl:None,process_role:"client".into(),dedicated_guest_qualified:false,
        allow_unverified_cursor_gate:false,flight_transport_enabled:false,vein_selected:false,vein_settings_path:None}}
}
pub fn plan(c:&Contract)->Result<SelectedBootPlanV2> {
    let mut services=0;
    for name in &c.boot_services {
        let bit=NAMES.iter().find(|(known,_)|known==name).context("unknown selected startup service")?.1;
        ensure!(services&bit==0,"duplicate selected startup service");services|=bit;
    }
    let ordered:Vec<_>=NAMES.iter().filter(|(_,bit)|services&bit!=0).map(|(name,_)|*name).collect();
    ensure!(c.boot_services.iter().map(String::as_str).eq(ordered),"selected startup service order differs");
    let process_role=match c.selection.process_role.as_str(){"client"=>1,"dedicated-server"=>2,_=>anyhow::bail!("unknown selected process role")};
    let opt_ins=u32::from(c.selection.dedicated_guest_qualified)|
        (u32::from(c.selection.allow_unverified_cursor_gate)<<1)|(u32::from(c.selection.flight_transport_enabled)<<2);
    let p=SelectedBootPlanV2{struct_bytes:24,version:2,services,process_role,opt_ins,reserved:0};
    ensure!(p.valid(),"unsupported selected startup requirements/role/opt-ins");
    ensure!(c.overlay==(services&GRAPHICS!=0)&&c.first_person==(services&FIRST_PERSON!=0)&&
        c.selection.vein_selected==(services&VEIN!=0),"selected startup demand differs");
    ensure!(if c.selection.vein_selected{c.selection.vein_settings_path.as_deref()==Some(VEIN_INI)}else{c.selection.vein_settings_path.is_none()},"Vein requires its exact admitted canonical INI path");
    Ok(p)
}
pub fn flight_settings(values:&Value)->Result<(bool,bool)> {
    let values=values.as_object().context("Flight settings are missing or unresolved")?;
    let mut generic=false;
    for key in ["turnSpeed","moveSpeed","liftSpeed","descendSpeed"] {
        let value=values.get(key).and_then(Value::as_f64).context(format!("Flight {key} is missing/invalid; no default coercion"))?;
        ensure!(value.is_finite()&&(25.0..=400.0).contains(&value),"Flight {key} is not a supported finite percentage");
        generic|=value!=100.0;
    }
    let flight=values.get("unlimitedFlying").and_then(Value::as_bool).context("Flight F6 selection is missing/invalid")?;
    Ok((generic,flight))
}
pub(super) fn validate_environment(env:&mod_loader::ModEnvironment,c:&Contract)->Result<crate::native_dlls::IntendedNativePlan> {
    let p=plan(c)?;
    // The current managed-library composition is a client graph. Dedicated
    // server legacy modes remain unchanged; no client graph is reinterpreted.
    ensure!(p.process_role==1,"schema6 selected managed-library graph is client-only");
    let selected=super::projected_runtime_plan(env,c)?;
    let mut generic=false;let mut flight=false;
    if let Some(package)=env.mod_registry().get(FLIGHT_ID) {
        let info=package.info();
        if info.enabled&&info.target.supports_process(false)&&info.capabilities.contains(&mod_loader::Capability::Runtime) {
            ensure!(selected.iter().any(|m|m.info().id==FLIGHT_ID),"Flight package did not qualify for the selected runtime plan");
            (generic,flight)=flight_settings(&info.setting_values)?;
        }
    } else {
        ensure!(!env.mods_dir().as_std_path().join(FLIGHT_ID).try_exists()?,"installed Flight package was rejected; startup demand is unknown");
        ensure!(mod_loader::package_lookup::find_mod_package(env.game_dir().as_std_path(),FLIGHT_ID).is_none(),"Flight package could not be admitted");
    }
    let fp=selected.iter().any(|m|m.info().id=="shroudweave-first-person");
    let intended=crate::native_dlls::intended_native_plan_for(env.game_dir().as_std_path(),false,c.selection.xhl.as_ref()).map_err(anyhow::Error::msg)?;
    ensure!(intended.identity==c.dll_plan,"intended native plan differs from prepared composition");
    let mut map=fp;let mut overlay=false;
    for plugin in &intended.entries {
        if !plugin.declaration.state.is_enabled(){continue;}
        if let Some(package)=plugin.entry.mod_id.as_deref().and_then(|id|env.mod_registry().get(id)) {
            let info=package.info();
            if info.enabled&&info.target.supports_process(false)&&info.capabilities.contains(&mod_loader::Capability::RuntimeRegisterDll){
                map|=plugin.declaration.declares_service("shroudforge.map");
                overlay|=plugin.declaration.declares_service("shroudforge.overlay");
            }
        }
    }
    ensure!((p.services&GENERIC_HOOKS!=0)==generic,"generic hooks differ from actual selected shared-speed demand");
    ensure!((p.services&FLIGHT_SCALAR!=0)==flight,"Flight preparation differs from actual selected F6 demand");
    ensure!((p.services&FIRST_PERSON!=0)==fp&&(p.services&MAP_CAPTURE!=0)==map,"Map/FP preparation differs from actual selected consumers");
    ensure!((p.services&GRAPHICS!=0)==overlay,"graphics preparation differs from actual selected consumers");
    Ok(intended)
}
pub(super) fn validate_live_settings(c:&Contract,id:&str,metadata:&Value)->Result<()> {
    if id!=FLIGHT_ID{return Ok(());}
    let (generic,flight)=flight_settings(&metadata["settingValues"])?;
    let p=plan(c)?;
    ensure!(!generic||p.services&GENERIC_HOOKS!=0,"shared glider speeds are unsupported without prepared generic ECS hooks; prepare and restart");
    ensure!(!flight||p.services&FLIGHT_SCALAR!=0,"F6 requires prepared Flight services; prepare and restart");
    Ok(())
}
#[cfg(windows)]
pub(super) fn file_identity(file:&fs::File)->Result<(u64,[u8;16])> {
    use std::os::windows::io::AsRawHandle;
    #[repr(C)] struct FileIdInfo {volume:u64,id:[u8;16]}
    const _: [();24]=[();std::mem::size_of::<FileIdInfo>()];
    #[link(name="kernel32")]
    unsafe extern "system" {#[link_name="GetFileInformationByHandleEx"] fn get_file_information_by_handle_ex(handle:*mut std::ffi::c_void,class:u32,information:*mut std::ffi::c_void,size:u32)->i32;}
    // FileIdInfo18 supplies the full128-bit ReFS/NTFS identity of the actual
    // already-open input. The SDK layout is checked by the owned C++ fixture.
    let mut information=FileIdInfo{volume:0,id:[0;16]};
    ensure!(unsafe{get_file_information_by_handle_ex(file.as_raw_handle(),18,(&mut information as *mut FileIdInfo).cast(),24)}!=0,"could not identify admitted INI input lease");
    Ok((information.volume,information.id))
}
#[cfg(unix)]
pub(super) fn file_identity(file:&fs::File)->Result<(u64,[u8;16])> {
    use std::os::unix::fs::MetadataExt;let m=file.metadata()?;let mut id=[0;16];id[..8].copy_from_slice(&m.ino().to_le_bytes());Ok((m.dev(),id))
}
#[cfg(not(any(windows,unix)))]
pub(super) fn file_identity(_:&fs::File)->Result<(u64,[u8;16])> {anyhow::bail!("INI lease identity unsupported")}
pub(super) fn validate_ini_handles(files:&[(String,fs::File)])->Result<()> {
    let ini=files.iter().find(|(name,_)|name==VEIN_INI).context("missing admitted INI lease")?;
    let expected=file_identity(&ini.1)?;
    for (name,file) in files {
        ensure!(name==VEIN_INI||file_identity(file)?!=expected,"another admitted input aliases the Vein INI; lease cannot be released independently");
    }
    Ok(())
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn four_speed_defaults_are_actual_values_not_error_fallbacks() {
        let mut v=serde_json::json!({"turnSpeed":100,"moveSpeed":100,"liftSpeed":100,"descendSpeed":100,"unlimitedFlying":true});
        assert_eq!(flight_settings(&v).unwrap(),(false,true));
        for key in ["turnSpeed","moveSpeed","liftSpeed","descendSpeed"] {
            v[key]=150.into();assert_eq!(flight_settings(&v).unwrap(),(true,true));
            for invalid in [Value::Null,Value::String("100".into()),0.into(),401.into()] {v[key]=invalid;assert!(flight_settings(&v).is_err());}
            v[key]=100.into();
        }
        v.as_object_mut().unwrap().remove("moveSpeed");assert!(flight_settings(&v).is_err());
        assert!(flight_settings(&Value::Null).is_err());
    }
}
