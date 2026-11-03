; Private attribution CPU-state bridge. The resident per-site relay calls this
; routine without changing registers/flags. Its leaf unwind row returns through
; the genuine patched-site CALL; all native game calls remain in native code.
option casemap:none
EXTERN SF_SemanticNativeDispatch:PROC
EXTERN SF_SemanticXsaveMask:QWORD
EXTERN GetLastError:PROC
EXTERN SetLastError:PROC
EXTERN __chkstk:PROC
.code
SF_SemanticNativeThunk PROC FRAME
    pushfq
    .allocstack 8
    push rax
    .allocstack 8
    push rcx
    .allocstack 8
    push rdx
    .allocstack 8
    push rbx
    .pushreg rbx
    push rbp
    .pushreg rbp
    push rsi
    .pushreg rsi
    push rdi
    .pushreg rdi
    push r8
    .allocstack 8
    push r9
    .allocstack 8
    push r10
    .allocstack 8
    push r11
    .allocstack 8
    push r12
    .pushreg r12
    push r13
    .pushreg r13
    push r14
    .pushreg r14
    push r15
    .pushreg r15
    mov r12,rsp
    .setframe r12,0
    .endprolog
    ; Frame register remains valid through dynamic aligned XSAVE storage.
    mov eax,50c0h
    call __chkstk
    sub rsp,rax
    and rsp,-64
    lea r13,[rsp+80h]
    mov rdi,r13
    xor eax,eax
    mov ecx,800h
    cld
    rep stosq
    fxsave64 [r13]
    mov eax,DWORD PTR SF_SemanticXsaveMask
    mov edx,DWORD PTR SF_SemanticXsaveMask+4
    xsave64 [r13]
    call GetLastError
    mov [rsp+40h],eax
    mov rcx,r12
    mov rdx,r13
    call SF_SemanticNativeDispatch
    mov ecx,[rsp+40h]
    call SetLastError
    mov eax,DWORD PTR SF_SemanticXsaveMask
    mov edx,DWORD PTR SF_SemanticXsaveMask+4
    xrstor64 [r13]
    mov rsp,r12
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rdi
    pop rsi
    pop rbp
    pop rbx
    pop rdx
    pop rcx
    pop rax
    popfq
    ret
SF_SemanticNativeThunk ENDP
END
