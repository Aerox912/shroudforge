option casemap:none
EXTERN SF_MiningJobBridge:PROC
EXTERN SF_MiningTerraformingBridge:PROC
EXTERN SF_MiningQueryBridge:PROC
EXTERN SF_MiningEnumerationBridge:PROC
EXTERN SF_MiningRegionBridge:PROC
.code
PUBLIC SF_MiningJobEntry
SF_MiningJobEntry PROC FRAME
 sub rsp,38h
 .allocstack 38h
 .endprolog
 mov r8,rcx
 lea rcx,[rsp+38h]
 mov rdx,[rcx]
 call SF_MiningJobBridge
 add rsp,38h
 ret
SF_MiningJobEntry ENDP
PUBLIC SF_MiningTerraformingEntry
SF_MiningTerraformingEntry PROC FRAME
 sub rsp,38h
 .allocstack 38h
 .endprolog
 mov [rsp+20h],r8
 mov r9,rdx
 mov r8,rcx
 lea rcx,[rsp+38h]
 mov rdx,[rcx]
 call SF_MiningTerraformingBridge
 add rsp,38h
 ret
SF_MiningTerraformingEntry ENDP
PUBLIC SF_MiningQueryEntry
SF_MiningQueryEntry PROC FRAME
 sub rsp,68h
 .allocstack 68h
 .endprolog
 mov [rsp+20h],r8
 mov [rsp+28h],r9
 mov rax,[rsp+90h]
 mov [rsp+30h],rax
 mov rax,[rsp+98h]
 mov [rsp+38h],rax
 mov rax,[rsp+0a0h]
 mov [rsp+40h],rax
 mov rax,[rsp+0a8h]
 mov [rsp+48h],rax
 mov rax,[rsp+0b0h]
 mov [rsp+50h],rax
 mov rax,[rsp+0b8h]
 mov [rsp+58h],rax
 mov r9,rdx
 mov r8,rcx
 lea rcx,[rsp+68h]
 mov rdx,[rcx]
 call SF_MiningQueryBridge
 add rsp,68h
 ret
SF_MiningQueryEntry ENDP
PUBLIC SF_MiningEnumerationEntry
SF_MiningEnumerationEntry PROC FRAME
 sub rsp,48h
 .allocstack 48h
 .endprolog
 mov [rsp+20h],r8
 mov [rsp+28h],r9
 mov rax,[rsp+70h]
 mov [rsp+30h],rax
 mov rax,[rsp+78h]
 mov [rsp+38h],rax
 mov rax,[rsp+80h]
 mov [rsp+40h],rax
 mov r9,rdx
 mov r8,rcx
 lea rcx,[rsp+48h]
 mov rdx,[rcx]
 call SF_MiningEnumerationBridge
 add rsp,48h
 ret
SF_MiningEnumerationEntry ENDP
PUBLIC SF_MiningRegionEntry
SF_MiningRegionEntry PROC FRAME
 sub rsp,38h
 .allocstack 38h
 .endprolog
 mov [rsp+20h],r8
 mov [rsp+28h],r9
 mov rax,[rsp+60h]
 mov [rsp+30h],rax
 mov r9,rdx
 mov r8,rcx
 lea rcx,[rsp+38h]
 mov rdx,[rcx]
 call SF_MiningRegionBridge
 add rsp,38h
 ret
SF_MiningRegionEntry ENDP
END
