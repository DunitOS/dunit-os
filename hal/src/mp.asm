section .text
bits 64
extern hal_mp_stack_tops
extern hal_mp_cr3
extern hal_mp_ap_main
global hal_mp_switch_stack
hal_mp_switch_stack:
    cli
    lea rax,[rel hal_mp_stack_tops]
    mov rsp,[rax+rdi*8]
    and rsp,-16
    xor rbp,rbp
    mov rdi,rsi
    mov rax,[rel hal_mp_cr3]
    mov cr3,rax
    mov rax,cr0
    and rax,~((1<<2)|(1<<3))
    or rax,1<<1
    mov cr0,rax
    mov rax,cr4
    and rax,~(1<<16)
    or rax,(1<<9)|(1<<10)
    mov cr4,rax
    fninit
    call hal_mp_ap_main
.halt:
    cli
    hlt
    jmp .halt
