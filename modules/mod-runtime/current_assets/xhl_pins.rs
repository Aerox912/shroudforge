//! Canonical data-only XHL selection pins, shared with the private installer.
pub const CORE_SHA:&str="98bb1f5506f6d15097c55de274e8b8079e8b901fd3d7b934f6433dde14d3c052";
pub const GIFTING_PATH:&str="XHL-Unlimited-Gifting-Range/XHL-Unlimited-Gifting-Range.dll";
pub const GIFTING_SHA:&str="6049751069281134704447909cb7a34e13607309c5fa09b9ba63ea3bb8509280";
pub const GIFTING_BYTES:u64=142_848;
pub const PLUGINS:[(&str,&str);3]=[
    ("XHL-Remote-Altar-Close/XHL-Remote-Altar-Close.dll","a92ddfff748a8c80c7a9b6237cf27e0b2f82024fb7dccc1a733b1342e4ec0ed5"),
    ("XHL-Grappling-Hook-Pull-2x/XHL-Grappling-Hook-Pull-2x.dll","decd6ad92e7d490583559c13e0babc706aaf9a6b83c3cc5b1935e9a2347f3b20"),
    (GIFTING_PATH,GIFTING_SHA),
];
