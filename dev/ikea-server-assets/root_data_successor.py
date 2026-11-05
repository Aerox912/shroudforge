"""Private retained-root data substitution. No game reads or DLL execution.

The caller supplies six final dependency hashes after their own acceptance.
This does not establish an exact source build of the retained root DLL.
"""
import hashlib
import json
import pathlib
import re
import struct

ROOT_SHA = '241db2155d8e79abb8e7596975b952a4bc1bdb3a63ce5af2fe6fc8123d34b490'
MAP_SHA = '2965e241ce39b0d948b6a4196c342aa9ff8a750d70456c8839acbb69b7f310ac'
REPORT_SHA = 'e8d1613379561d94c83a4654b213cf07719209ca27df2fddd54e8f32626b5df0'
PINS = {
 'game': (266832,'d7baa768eb00aa895d9f5a1cd8747ccf66858c7b542f50983092b2e3261588ee'),
 'native': (266912,'62c8708e52602b561d2bb88c263ef7bcfa830e37f55ea7f8810c1ff2eeef649d'),
 'config': (266992,'8741675902ea3b136969d5f59cde26bc231024fad82054af2e77726648d85cdb'),
 'supervisor': (267072,'7e09be6022d2aa05f335ff994c1aa3d4b1486a8fc8edad359991449e8a949599'),
 'chainloader': (267152,'e38a9dfe388138f67c7e4581df095320c8ee352b439944fa4fd9108350bc5a26'),
 'catalog': (267232,'4f2a4201e2e22a36c6c8c87af4c5ff2262e541f66a6227e1a380aee17f03adc8'),
}
NAMES = ['SPAWN_CREATE','PLAYERS_PUMP','PLAYER_PERMISSIONS','IS_ENEMY_VALUES','IS_ENEMY_PTRS','IS_FRIEND_VALUES','IS_FRIEND_OPTS']
sha = lambda b: hashlib.sha256(b).hexdigest()

def require(condition, message):
    if not condition: raise ValueError(message)

def prepare(root, mapping, dependencies):
    require(len(root)==375296 and sha(root)==ROOT_SHA, 'retained root differs')
    require(sha(mapping)==MAP_SHA, 'site mapping differs')
    m=json.loads(mapping)
    require(m['reportSha256']==REPORT_SHA, 'capture differs')
    require(set(dependencies)==set(PINS), 'six explicit final dependency hashes required')
    require(dependencies['game']=='f0d824be7fcd48ebfe2458ff5c28da26060c2074e3ec1fcac587e5a1d508aeb2', 'game differs')
    pe=struct.unpack_from('<I',root,60)[0];coff=pe+4;opt=coff+20
    count=struct.unpack_from('<H',root,coff+2)[0];size=struct.unpack_from('<H',root,coff+16)[0]
    sections=[]
    for i in range(count):
        p=opt+size+i*40;length,offset=struct.unpack_from('<II',root,p+16)
        sections.append((root[p:p+8].rstrip(b'\0'),offset,length,struct.unpack_from('<I',root,p+36)[0]))
    def data_range(offset,length):
        owners=[s for s in sections if s[1]<=offset and offset+length<=s[1]+s[2]]
        require(len(owners)==1 and owners[0][0]==b'.rdata' and owners[0][3]&0x20000000==0,'not read-only data section')
    output=bytearray(root);changes=[];allowed=set()
    def change(label,offset,before,after):
        require(len(before)==len(after) and root[offset:offset+len(before)]==before,'preimage differs: '+label)
        data_range(offset,len(before));allowed.update(range(offset,offset+len(before)))
        output[offset:offset+len(before)]=after
        changes.append(dict(label=label,fileOffset=offset,before=before.hex(),after=after.hex()))
    for i,name in enumerate(NAMES):
        matches=[s for s in m['sites'] if s['role']=='godmode_'+name]
        require(len(matches)==1,'missing site');s=matches[0];rows=s['rootTableRecords'];offset=264648+i*200
        require(len(rows)==1 and rows[0]['fileOffsets']==[offset], 'site ordering differs')
        row=rows[0];record=root[offset:offset+200]
        require(sha(record)==row['recordSha256'], 'whole record differs')
        old,hook,length=struct.unpack_from('<III',record)
        signature=bytes.fromhex(s['currentSignatureHex'])
        require((old,hook,length)==(row['rva'],row['offset'],row['length']), 'record fields differ')
        require(s['maskOffsets']==[] and length==len(signature) and record[12:12+length]==signature, 'literal signature differs')
        require(s['currentSignatureSha256']==sha(signature) and s['signatureSha256']==sha(signature),'signature digest differs')
        require(s['hookOffset']==hook and s['currentHookRva']==s['currentRva']+hook and s['hookBytes']==14,'hook layout differs')
        require(signature[hook:hook+14].hex()==s['currentPreimage']==s['sourceHookPreimage'],'hook bytes differ')
        change(name,offset,struct.pack('<I',old),struct.pack('<I',s['currentRva']))
    for name,(offset,old) in PINS.items():
        new=dependencies[name];require(isinstance(new,str) and re.fullmatch('[0-9a-f]{64}',new),'invalid dependency hash')
        require(root[offset+64]==0,'pin terminator differs')
        change(name,offset,old.encode(),new.encode())
    actual={i for i,(a,b) in enumerate(zip(root,output)) if a!=b}
    require(actual<=allowed and len(output)==len(root),'unauthorized output difference')
    for _,offset,length,flags in sections:
        if flags&0x20000000:require(output[offset:offset+length]==root[offset:offset+length],'executable section changed')
    return bytes(output),dict(status='PRIVATE_ROOT_DATA_SUCCESSOR_PENDING_COMPOSITION',inputSha256=ROOT_SHA,outputSha256=sha(output),bytes=len(output),mappingSha256=MAP_SHA,captureSha256=REPORT_SHA,dependencies=dependencies,changes=changes,changedByteCount=len(actual),executableSectionsUnchanged=True,nativeExecuted=False,exactSourceBuild=False,liveCompatibilityAccepted=False)

if __name__=='__main__':
    import argparse
    p=argparse.ArgumentParser();p.add_argument('root');p.add_argument('mapping');p.add_argument('dependencies');p.add_argument('output')
    args=p.parse_args();out=pathlib.Path(args.output)
    require(not out.exists(),'fresh output directory required')
    binary,receipt=prepare(pathlib.Path(args.root).read_bytes(),pathlib.Path(args.mapping).read_bytes(),json.loads(pathlib.Path(args.dependencies).read_bytes()))
    out.mkdir()
    with (out/'shroudweave_godmode.dll').open('xb') as f:f.write(binary)
    with (out/'RECEIPT.json').open('x',encoding='utf-8') as f:json.dump(receipt,f,indent=2)
    print(json.dumps(receipt))
