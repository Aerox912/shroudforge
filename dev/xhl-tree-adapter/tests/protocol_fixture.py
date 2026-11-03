"""RB65 E-only inert adapter subprocess test. DLLs are copied/hashed, never loaded."""
import ctypes as c,hashlib,json,os,pathlib,shutil,subprocess,sys,time
from ctypes import wintypes as w
k=c.WinDLL('kernel32',use_last_error=True)
k.CreateFileW.argtypes=[w.LPCWSTR,w.DWORD,w.DWORD,c.c_void_p,w.DWORD,w.DWORD,c.c_void_p];k.CreateFileW.restype=w.HANDLE
k.GetFileInformationByHandleEx.argtypes=[w.HANDLE,c.c_int,c.c_void_p,w.DWORD];k.GetFileInformationByHandleEx.restype=w.BOOL
k.CloseHandle.argtypes=[w.HANDLE]
class FileId(c.Structure):_fields_=[('volume',c.c_ulonglong),('id',c.c_ubyte*16)]
assert c.sizeof(FileId)==24
def identity(path):
    h=k.CreateFileW(str(path),0x80,7,None,3,0x02000000|0x00200000,None)
    if h==c.c_void_p(-1).value:raise c.WinError(c.get_last_error())
    try:
        fid=FileId()
        if not k.GetFileInformationByHandleEx(h,18,c.byref(fid),24):raise c.WinError(c.get_last_error())
        return {'volume':fid.volume,'id':list(fid.id)}
    finally:k.CloseHandle(h)
def sha(b):return hashlib.sha256(b).hexdigest()
exe=pathlib.Path(sys.argv[1]).resolve();base=pathlib.Path(sys.argv[2]).resolve();evidence=pathlib.Path(sys.argv[3]).resolve()
assert str(base).startswith('E:\\Scratch\\sf177-xhl-adapter-') and not base.exists()
inputs={
 'xhl-core.dll':pathlib.Path(r'E:\Scratch\sf177-xhl-core-probe-20261103\static\dinput8.dll'),
 'mods/XHL-Remote-Altar-Close/XHL-Remote-Altar-Close.dll':pathlib.Path(r'E:\Scratch\sf177-xhl-core-probe-20261103\cases\rb44-real-plugins\game\core\mods\XHL-Remote-Altar-Close\XHL-Remote-Altar-Close.dll'),
 'mods/XHL-Grappling-Hook-Pull-2x/XHL-Grappling-Hook-Pull-2x.dll':pathlib.Path(r'E:\Scratch\sf177-xhl-core-probe-20261103\cases\rb44-real-plugins\game\core\mods\XHL-Grappling-Hook-Pull-2x\XHL-Grappling-Hook-Pull-2x.dll'),
 'mods/XHL-Unlimited-Gifting-Range/XHL-Unlimited-Gifting-Range.dll':pathlib.Path(r'E:\Build\gifting-current-test-20261103\Release\XHL-Unlimited-Gifting-Range.dll')}
results=[]
for terminal in ['commit','rollback']:
    case=base/terminal;source=case/'source';parent=case/'game/mods';source.mkdir(parents=True);parent.mkdir(parents=True);files={}
    for name,path in inputs.items():
        dst=source/name;dst.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(path,dst)
        if name!='xhl-core.dll':(dst.parent/'native-plugin.ini').write_bytes(b'[Plugin]\r\nEnabled=true\r\nDll='+dst.name.encode()+b'\r\n')
    for path in source.rglob('*'):
        if path.is_file():files[path.relative_to(source).as_posix()]={'source':path.relative_to(source).as_posix(),'sourceIdentity':identity(path),'bytes':path.stat().st_size,'sha256':sha(path.read_bytes())}
    chain=b'[Options]\r\nEnabled=0\r\n';files['dinput8-chain.ini']={'source':None,'sourceIdentity':None,'bytes':len(chain),'sha256':sha(chain)}
    plan={'schemaVersion':1,'transaction':'rb65-protocol-'+terminal,'sourceRoot':str(source),'sourceRootIdentity':identity(source),'targetParent':str(parent),'targetParentIdentity':identity(parent),'files':files}
    raw=json.dumps(plan,separators=(',',':')).encode();planfile=case/'plan.json';planfile.write_bytes(raw)
    proc=subprocess.Popen([str(exe),str(planfile),sha(raw)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,cwd=case)
    try:
        prepared=json.loads(proc.stdout.readline());assert prepared['outcome']=='PREPARED_TREE',prepared
        receipt=prepared['receipt'];assert len(receipt['files'])==8 and len(receipt['directories'])==4
        assert receipt['container']['id']==identity(parent/'xhl-core')['id']
        for d in receipt['directories']:assert d['id']==identity(parent/'xhl-core/mods'/d['relative'])['id']
        # Commit requires the receipt actually issued by this process. A bad ack
        # neither exits nor releases its file leases.
        ack={'transaction':plan['transaction'],'approvedPlanSha256':sha(raw),'preparedReceiptSha256':'0'*64}
        proc.stdin.write(json.dumps({'command':'commit','ack':ack})+'\n');proc.stdin.flush()
        refused=json.loads(proc.stdout.readline());assert refused['outcome']=='COMMIT_REFUSED' and refused['ownerRetained']
        try:(parent/'xhl-core/xhl-core.dll').write_bytes(b'forbidden');raise AssertionError('lease released before ack')
        except PermissionError:pass
        if terminal=='commit':ack['preparedReceiptSha256']=prepared['preparedReceiptSha256'];command={'command':'commit','ack':ack}
        else:command={'command':'rollback'}
        proc.stdin.write(json.dumps(command)+'\n');proc.stdin.flush();result=json.loads(proc.stdout.readline());proc.stdin.close()
        assert proc.wait(timeout=15)==0,proc.stderr.read()
        assert result['outcome']==('COMMITTED_SEALED_TREE' if terminal=='commit' else 'ACL_RESTORED_NEW_TREE_RETAINED')
        assert (parent/'xhl-core').exists()
        results.append({'terminal':terminal,'prepared':prepared,'refused':refused,'result':result,'exit_code':proc.returncode})
    finally:
        if proc.poll()is None:proc.kill();proc.wait()
evidence.write_text(json.dumps({'task_id':'RB65','own_inert_adapter_only':True,'cases':results},indent=2))
shutil.rmtree(base)
print('2 finite adapter protocol cases passed; actual IDs, bad-ack lease retention, commit and explicit retained-tree rollback verified')
