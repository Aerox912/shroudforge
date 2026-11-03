option casemap:none
EXTERN SF_SemanticNativeThunk:PROC
EXTERN SF_SemanticXsaveMask:QWORD
.code
; The caller supplies an OWNED synthetic emitted relay, never archived code.
SF_SemanticOwnedInvokeRelay PROC FRAME
    push rbx
    .pushreg rbx
    sub rsp,20h
    .allocstack 20h
    .endprolog
    mov rbx,rdx
    lea rax,[rsp+20h]
    mov [rbx],rax
    call rcx
    add rsp,20h
    pop rbx
    ret
SF_SemanticOwnedInvokeRelay ENDP
SF_SemanticOwnedRelayShim PROC FRAME
    .endprolog
    call SF_SemanticNativeThunk
    ret
SF_SemanticOwnedRelayShim ENDP
SF_SemanticOwnedClobber PROC FRAME
    .endprolog
    pxor xmm0,xmm0
    test DWORD PTR SF_SemanticXsaveMask,4
    jz no_avx
    vpxor ymm0,ymm0,ymm0
no_avx:
    xor eax,eax
    xor ecx,ecx
    xor edx,edx
    xor r8d,r8d
    xor r9d,r9d
    xor r10d,r10d
    xor r11d,r11d
    clc
    ret
SF_SemanticOwnedClobber ENDP
SF_SemanticOwnedThunkFixture PROC FRAME
    push rbx
    .pushreg rbx
    sub rsp,20h
    .allocstack 20h
    .endprolog
    mov rbx,123456789abcdef0h
    mov rax,1122334455667788h
    mov rcx,8877665544332211h
    mov rdx,1020304050607080h
    mov r8,0fedcba987654321h
    mov r9,7654321012345678h
    mov r10,0011223344556677h
    mov r11,7766554433221100h
    pcmpeqd xmm0,xmm0
    test DWORD PTR SF_SemanticXsaveMask,4
    jz no_avx_initialize
    vpcmpeqd ymm0,ymm0,ymm0
no_avx_initialize:
    stc
    call SF_SemanticOwnedRelayShim
    jnc failed
    movd edx,xmm0
    cmp edx,-1
    jne failed
    test DWORD PTR SF_SemanticXsaveMask,4
    jz no_avx_verify
    vextractf128 xmm1,ymm0,1
    movd edx,xmm1
    cmp edx,-1
    jne failed
no_avx_verify:
    mov rdx,1122334455667788h
    cmp rax,rdx
    jne failed
    mov rdx,123456789abcdef0h
    cmp rbx,rdx
    jne failed
    mov eax,1
    jmp done
failed:
    xor eax,eax
done:
    add rsp,20h
    pop rbx
    ret
SF_SemanticOwnedThunkFixture ENDP
END
