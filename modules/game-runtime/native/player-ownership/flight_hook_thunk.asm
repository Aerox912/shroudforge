; Internal exact-site scalar operand bridge. Original MOVSS clears XMM0[127:32]
; and preserves flags and every other register. Save all enabled XCR0 state,
; including upper YMM/ZMM lanes, rather than assuming a C++ call preserves it.
; Must only be published after PrepareScalarCpuState and scalar-address setup.
option casemap:none
EXTERN SF_FlightSelectScalar:PROC
EXTERN SF_FlightXsaveMask:QWORD
EXTERN SF_FlightScalarAddress:QWORD
EXTERN GetLastError:PROC
EXTERN SetLastError:PROC
EXTERN __chkstk:PROC
.code
SF_FlightScalarThunk PROC FRAME
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
    ; Dynamic, probed, 64-byte aligned state storage. r12 remains the unwind
    ; frame and is preserved by ordinary Windows-x64 callees.
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
    mov eax,DWORD PTR SF_FlightXsaveMask
    mov edx,DWORD PTR SF_FlightXsaveMask+4
    xsave64 [r13]
    mov rax,[r12+50h]
    mov [rsp+20h],rax
    lea rax,[r12+88h]
    mov [rsp+28h],rax
    mov rax,[r12+80h]
    sub rax,5
    mov [rsp+30h],rax
    mov eax,[r13+0b4h]
    mov [rsp+38h],eax
    call GetLastError
    mov [rsp+40h],eax
    mov rax,SF_FlightScalarAddress
    mov eax,[rax]
    mov [rsp+3ch],eax
    lea rcx,[rsp+20h]
    call SF_FlightSelectScalar
    mov [rsp+44h],eax
    mov ecx,[rsp+40h]
    call SetLastError
    mov eax,[rsp+44h]
    mov [r13+0a0h],eax
    mov DWORD PTR [r13+0a4h],0
    mov QWORD PTR [r13+0a8h],0
    ; The operand load makes SSE state non-initial even if it was initial at
    ; entry. XSAVE's standard header must request restoration of that state.
    or QWORD PTR [r13+200h],2
    mov eax,DWORD PTR SF_FlightXsaveMask
    mov edx,DWORD PTR SF_FlightXsaveMask+4
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
SF_FlightScalarThunk ENDP
END
