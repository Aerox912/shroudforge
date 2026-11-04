//! Exact original-loader profiles shared by runtime and offline assembly.
pub struct Profile {
    pub id: &'static str,
    pub version: u32,
    pub definition_sha256: &'static str,
    pub host_bytes: u64,
    pub host_sha256: &'static str,
    pub ticket_kind: u32,
    pub scan_inputs: [(&'static str, u64, &'static str); 4],
}
pub const LEGACY: Profile = Profile {
    id: "greeneye-entry-continuation-v1", version: 1,
    definition_sha256: "dae3fcd2185451c97d1e9f4d41676f6748eaecc8cb194eb9a438c6a7f5d038d2",
    host_bytes: 288256,
    host_sha256: "7b97b41bd89d46528db9de4fb63b4f9fb32878192ade605c6da5e39da003687f",
    ticket_kind: 1,
    scan_inputs: [
        ("greeneye/find/mod.json",265,"911775560b0a23fd8f8571c9f75b108562f358e6653752a246ebcfe917ac1293"),
        ("greeneye/find/find.dll",519680,"0c25d97f36fb35b711e5e5240f53ea745c6077d2adf04420055dba049b03b299"),
        ("greeneye/map-teleport/mod.json",279,"ca543192371fd1afc1537f12425ab1996303180b25c67d8e21184b1c5b6e92d3"),
        ("greeneye/map-teleport/map-teleport.dll",444928,"9138d31acab0623a63824390ab0418fb18183709df9758f7f4c89670cc836c78"),
    ],
};
pub const LATEST: Profile = Profile {
    id: "greeneye-entry-continuation-v2", version: 2,
    definition_sha256: "a0c13b2eaa8baf923be3e6590f02df803b1f4ab9822127aba1f342fac77199bc",
    host_bytes: 294912,
    host_sha256: "973a2284ba4faf24e55c9e4b165c136c81dec4d3b1706b3f21f9cbd314c26eed",
    ticket_kind: 2,
    scan_inputs: [
        ("greeneye/find/mod.json",265,"937c32f41858bae9b2fa9fceef1ce53c01bac39cd4e048c13c20545a3235e512"),
        ("greeneye/find/find.dll",522752,"1660249c2312258600f7bd7c9ce2469e3bc8c7e27f71896d77fb77c9e50669b0"),
        ("greeneye/map-teleport/mod.json",280,"17e852be7d064260a17412df91f3b6b604ebb96e16c94d1c56d025ad5ed34b41"),
        ("greeneye/map-teleport/map-teleport.dll",448512,"438610671f1dfe0ce4b9d24f45a878664a5d9d504e6f797e21b44aa3d39a6630"),
    ],
};
pub fn for_host(bytes: u64, sha256: &str) -> Option<&'static Profile> {
    [&LEGACY, &LATEST].into_iter().find(|p| p.host_bytes == bytes && p.host_sha256 == sha256)
}
pub fn for_route(id: &str, version: u32, definition: &str) -> Option<&'static Profile> {
    [&LEGACY, &LATEST].into_iter().find(|p| p.id == id && p.version == version && p.definition_sha256 == definition)
}
