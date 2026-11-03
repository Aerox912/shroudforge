"""Inspect only built fixture/proxy artifacts as data; never load a module."""
import hashlib,json,pathlib,sys
import pefile
out=pathlib.Path(sys.argv[1]); result={}
names=['DirectInput8Create','DllCanUnloadNow','DllGetClassObject','DllRegisterServer','DllUnregisterServer','GetdfDIJoystick']
for variant in ['sf','eml']:
    path=out/f'{variant}-dinput8.dll'; pe=pefile.PE(str(path)); exports={s.name.decode():s for s in pe.DIRECTORY_ENTRY_EXPORT.symbols if s.name}
    for ordinal,name in enumerate(names,1):
        assert exports[name].ordinal==ordinal,(variant,name,'original ordinal changed')
        if variant=='eml' or ordinal!=1:
            assert pe.get_data(exports[name].address,2)==b'\xff\x25',(variant,name,'system jump changed')
    imports={d.dll.decode().lower():[{'name':i.name.decode() if i.name else None,'ordinal':i.ordinal} for i in d.imports] for d in pe.DIRECTORY_ENTRY_IMPORT}
    if variant=='sf':
        assert 'ShroudforgeSetDinput8Provider' in exports
        assert any(i['ordinal']==184 for i in imports.get('winmm.dll',[])),'missing actual ShroudforgeStart ordinal import'
    else:
        assert 'ShroudforgeSetDinput8Provider' not in exports
        assert 'winmm.dll' not in imports,'featureless EML acquired SF dependency'
    result[variant]={'sha256':hashlib.sha256(path.read_bytes()).hexdigest(),'bytes':path.stat().st_size,
        'original_exports':{n:exports[n].ordinal for n in names},'setter':'ShroudforgeSetDinput8Provider' in exports,'imports':imports,
        'system_forward_jump_checks':6 if variant=='eml' else 5}
(out/'PE-EVIDENCE.json').write_text(json.dumps(result,indent=2))
print('SF/EML exports, 11 system jumps, typed setter and static winmm ordinal184 import verified')
