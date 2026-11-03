use std::{fs::OpenOptions,io::{self,BufRead,Read,Write},os::windows::fs::{OpenOptionsExt,MetadataExt}};
use serde::Deserialize;
use serde_json::json;
use sf_xhl_tree_adapter::{Install,CommitAck};
#[derive(Deserialize)]
#[serde(tag="command",rename_all="kebab-case",deny_unknown_fields)]
enum Command {Commit{ack:CommitAck},Rollback,Status}
fn output(value:serde_json::Value)->io::Result<()> {let mut out=io::stdout().lock();serde_json::to_writer(&mut out,&value)?;writeln!(out)?;out.flush()}
fn line(input:&mut impl BufRead)->io::Result<Option<Vec<u8>>> {
    let mut result=Vec::new();loop {let available=input.fill_buf()?;if available.is_empty(){return if result.is_empty(){Ok(None)}else{Err(io::Error::other("unterminated command"))};}
        let n=available.iter().position(|c|*c==b'\n').map(|i|i+1).unwrap_or(available.len());
        if result.len()+n>65536{return Err(io::Error::other("command exceeds bound"));}result.extend_from_slice(&available[..n]);input.consume(n);
        if result.last()==Some(&b'\n'){return Ok(Some(result));}}
}
fn run()->Result<(),String> {
    let args:Vec<_>=std::env::args_os().collect();if args.len()!=3{return Err("usage: sf-xhl-tree-adapter <approved-plan.json> <approved-plan-sha256>".into());}
    // Keep the exact caller-supplied plan file leased; no inferred/default digest.
    let mut plan=OpenOptions::new().read(true).share_mode(1).custom_flags(0x00200000).open(&args[1]).map_err(|e|e.to_string())?;
    let metadata=plan.metadata().map_err(|e|e.to_string())?;if !metadata.is_file()||metadata.file_attributes()&0x400!=0{return Err("plan must be a regular non-reparse file".into());}
    let mut bytes=Vec::new();(&mut plan).take(1024*1024+1).read_to_end(&mut bytes).map_err(|e|e.to_string())?;
    let expected=args[2].to_str().ok_or("nonUTF8 approved digest")?;
    let mut owner=match Install::prepare(&bytes,expected){Ok(owner)=>owner,Err(failure)=>{
        output(json!({"outcome":"PREPARE_FAILED","error":failure.error,"ownerRetained":failure.owner.is_some()})).map_err(|e|e.to_string())?;
        match failure.owner{Some(owner)=>*owner,None=>return Err("prepare refused without creating a target".into())}
    }};
    if let Some(receipt)=owner.receipt(){output(json!({"outcome":"PREPARED_TREE","preparedReceiptSha256":receipt.digest(),"receipt":receipt})).map_err(|e|e.to_string())?;}
    let mut input=io::stdin().lock();loop {
        let Some(bytes)=line(&mut input).map_err(|e|e.to_string())?else{return Err("EOF before terminal transaction outcome; uncommitted new paths may require separate maintenance".into());};
        let command=match serde_json::from_slice::<Command>(&bytes){Ok(c)=>c,Err(e)=>{output(json!({"outcome":"COMMAND_REFUSED","error":e.to_string(),"ownerRetained":true})).map_err(|e|e.to_string())?;continue;}};
        match command {
            Command::Status=>{output(json!({"outcome":"OWNER_RETAINED","receipt":owner.receipt()})).map_err(|e|e.to_string())?;},
            Command::Commit{ack}=>match owner.commit(&ack){Ok(receipt)=>{output(json!({"outcome":"COMMITTED_SEALED_TREE","receipt":receipt,"permanentByteImmutability":false})).map_err(|e|e.to_string())?;return Ok(());},Err(e)=>{output(json!({"outcome":"COMMIT_REFUSED","error":e,"ownerRetained":true})).map_err(|e|e.to_string())?;}},
            Command::Rollback=>match owner.rollback(){Ok(receipt)=>{output(serde_json::to_value(receipt).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;return Ok(());},Err(e)=>{output(json!({"outcome":"ROLLBACK_INCOMPLETE","error":e,"ownerRetained":true})).map_err(|e|e.to_string())?;}},
        }
    }
}
fn main(){if let Err(e)=run(){eprintln!("{e}; no automatic launch, cleanup or restart recovery");std::process::exit(2);}}
