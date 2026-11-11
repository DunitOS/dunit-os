section .text
bits 64
%include "hal/src/cpu_local.inc"

extern syscall_handler

%define USER_CODE_SELECTOR 0x1B
%define USER_DATA_SELECTOR 0x23

global syscall_entry
syscall_entry:
    swapgs
    lfence
    ; SYSCALL leaves the user return RIP in RCX and RFLAGS in R11.
    ; It does not switch stacks, so use the selected process kernel stack when
    ; the kernel has installed one, otherwise fall back to a private bootstrap
    ; syscall stack. Entry scratch and selected/escape stacks are CPU-local;
    ; this mechanism does not itself enable parallel task dispatch.
    mov [gs:CPU_SYSCALL_USER_RSP], rsp
    mov [gs:CPU_SYSCALL_SAVED_RAX], rax
    mov [gs:CPU_SYSCALL_SAVED_RBX], rbx
    mov [gs:CPU_SYSCALL_SAVED_RCX], rcx
    mov [gs:CPU_SYSCALL_SAVED_RDX], rdx
    mov [gs:CPU_SYSCALL_SAVED_RSI], rsi
    mov [gs:CPU_SYSCALL_SAVED_RDI], rdi
    mov [gs:CPU_SYSCALL_SAVED_RBP], rbp
    mov [gs:CPU_SYSCALL_SAVED_R8], r8
    mov [gs:CPU_SYSCALL_SAVED_R9], r9
    mov [gs:CPU_SYSCALL_SAVED_R10], r10
    mov [gs:CPU_SYSCALL_SAVED_R11], r11
    mov [gs:CPU_SYSCALL_SAVED_R12], r12
    mov [gs:CPU_SYSCALL_SAVED_R13], r13
    mov [gs:CPU_SYSCALL_SAVED_R14], r14
    mov [gs:CPU_SYSCALL_SAVED_R15], r15
    mov rax, [gs:CPU_USER_FPU_STATE]
    test rax, rax
    jz .fpu_saved
    fxsave [rax]
.fpu_saved:
    mov rsp, [gs:CPU_SYSCALL_SELECTED_STACK_TOP]
    test rsp, rsp
    jnz .stack_selected
    mov rsp, [gs:CPU_BOOTSTRAP_STACK_TOP]
.stack_selected:
    and rsp, -16
    mov rax, [gs:CPU_SYSCALL_SAVED_RAX]

    push rcx
    push r11
    push rdi
    push rsi
    push rdx
    push r10
    push r8
    push r9

    ; Userspace ABI:
    ;   rax=syscall number, rdi/rsi/rdx/r10/r8/r9=args 0..5
    ; Rust extern "C" ABI:
    ;   rdi/rsi/rdx/rcx/r8/r9=args 0..5, additional args on stack
    ; The first Rust argument is the syscall number, so userspace arg5 is
    ; passed as the first stack argument.
    mov r10, [rsp]       ; user arg5
    mov r9, [rsp + 8]    ; user arg4
    mov r8, [rsp + 16]   ; user arg3
    mov rcx, [rsp + 24]  ; user arg2
    mov rdx, [rsp + 32]  ; user arg1
    mov rsi, [rsp + 40]  ; user arg0
    mov rdi, rax

    sub rsp, 8
    push r10

    call syscall_handler

    add rsp, 16

    pop r9
    pop r8
    pop r10
    pop rdx
    pop rsi
    pop rdi
    pop r11
    pop rcx

    cmp rax, [rel syscall_smoke_return_magic]
    jne .return_to_user
    cmp qword [gs:CPU_SYSCALL_SMOKE_ACTIVE], 1
    jne .return_to_user
    mov qword [gs:CPU_SYSCALL_SMOKE_ACTIVE], 0
    mov rsp, [gs:CPU_SYSCALL_SMOKE_KERNEL_RSP]
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbp
    pop rbx
    sti
    ret

.return_to_user:
    ; Return with IRETQ instead of SYSRET because the current GDT layout has
    ; user code before user data, which is not compatible with SYSRET's fixed
    ; selector derivation rules.
    push qword USER_DATA_SELECTOR
    push qword [gs:CPU_SYSCALL_USER_RSP]
    push r11
    push qword USER_CODE_SELECTOR
    push rcx
    push rax
    mov rax, [gs:CPU_USER_FPU_STATE]
    test rax, rax
    jz .fpu_restored
    fxrstor [rax]
.fpu_restored:
    pop rax
    swapgs
    iretq

global set_user_fpu_state
set_user_fpu_state:
    mov [gs:CPU_USER_FPU_STATE], rdi
    ret

global set_fs_base
set_fs_base:
    mov rax, rdi
    mov rdx, rax
    shr rdx, 32
    mov ecx, 0xC0000100
    wrmsr
    ret

global get_fs_base
get_fs_base:
    mov ecx, 0xC0000100
    rdmsr
    shl rdx, 32
    or rax, rdx
    ret

global syscall_init
syscall_init:
    ; EFER.SCE = enable SYSCALL/SYSRET instructions.
    mov ecx, 0xC0000080
    rdmsr
    or eax, 1
    wrmsr

    ; STAR: kernel CS is 0x08. The upper user selector field is left matching
    ; the current GDT even though syscall_entry returns via IRETQ.
    mov ecx, 0xC0000081
    rdmsr
    mov eax, 0
    mov edx, 0x00180008
    wrmsr

    ; LSTAR = syscall entry point.
    mov ecx, 0xC0000082
    lea rax, [rel syscall_entry]
    mov rdx, rax
    shr rdx, 32
    wrmsr

    ; SFMASK: clear IF and DF for the kernel C/Rust ABI. SYSCALL saves
    ; the original user flags in R11; IRET restores those unchanged.
    mov ecx, 0xC0000084
    mov eax, 0x00000600
    xor edx, edx
    wrmsr

    ret

global syscall_set_kernel_stack_top
syscall_set_kernel_stack_top:
    and rdi, -16
    mov [gs:CPU_SYSCALL_SELECTED_STACK_TOP], rdi
    ret

global syscall_reset_kernel_stack
syscall_reset_kernel_stack:
    mov qword [gs:CPU_SYSCALL_SELECTED_STACK_TOP], 0
    ret

global syscall_get_escape_kernel_rsp
syscall_get_escape_kernel_rsp:
    mov rax, [gs:CPU_SYSCALL_SMOKE_KERNEL_RSP]
    ret

global syscall_get_escape_active
syscall_get_escape_active:
    mov rax, [gs:CPU_SYSCALL_SMOKE_ACTIVE]
    ret

global syscall_restore_escape_state
syscall_restore_escape_state:
    mov [gs:CPU_SYSCALL_SMOKE_KERNEL_RSP], rdi
    mov [gs:CPU_SYSCALL_SMOKE_ACTIVE], rsi
    ret

global syscall_capture_user_context
syscall_capture_user_context:
    ; Rust extern "C":
    ;   rdi = *mut CpuContext
    ;   rsi = resumed syscall return value
    mov [rdi + 0], rsi
    mov rax, [gs:CPU_SYSCALL_SAVED_RBX]
    mov [rdi + 8], rax
    mov rax, [gs:CPU_SYSCALL_SAVED_RCX]
    mov [rdi + 16], rax
    mov rax, [gs:CPU_SYSCALL_SAVED_RDX]
    mov [rdi + 24], rax
    mov rax, [gs:CPU_SYSCALL_SAVED_RSI]
    mov [rdi + 32], rax
    mov rax, [gs:CPU_SYSCALL_SAVED_RDI]
    mov [rdi + 40], rax
    mov rax, [gs:CPU_SYSCALL_SAVED_RBP]
    mov [rdi + 48], rax
    mov rax, [gs:CPU_SYSCALL_USER_RSP]
    mov [rdi + 56], rax
    mov rax, [gs:CPU_SYSCALL_SAVED_R8]
    mov [rdi + 64], rax
    mov rax, [gs:CPU_SYSCALL_SAVED_R9]
    mov [rdi + 72], rax
    mov rax, [gs:CPU_SYSCALL_SAVED_R10]
    mov [rdi + 80], rax
    mov rax, [gs:CPU_SYSCALL_SAVED_R11]
    mov [rdi + 88], rax
    mov rax, [gs:CPU_SYSCALL_SAVED_R12]
    mov [rdi + 96], rax
    mov rax, [gs:CPU_SYSCALL_SAVED_R13]
    mov [rdi + 104], rax
    mov rax, [gs:CPU_SYSCALL_SAVED_R14]
    mov [rdi + 112], rax
    mov rax, [gs:CPU_SYSCALL_SAVED_R15]
    mov [rdi + 120], rax
    mov rax, [gs:CPU_SYSCALL_SAVED_RCX]
    mov [rdi + 128], rax
    mov rax, [gs:CPU_SYSCALL_SAVED_R11]
    mov [rdi + 136], rax
    ret

global syscall_restart_user_context
syscall_restart_user_context:
    ; Rust extern "C": rdi = *mut CpuContext.
    ; Like syscall_capture_user_context, but set up to RE-EXECUTE the syscall on
    ; resume instead of returning past it: rax = the original syscall number and
    ; rip = the `syscall` instruction itself (saved return rip - 2 bytes). Used
    ; by blocking syscalls (pipe read/write) that must re-run and recompute their
    ; result on wake, not return a value captured at block time.
    mov rax, [gs:CPU_SYSCALL_SAVED_RAX]
    mov [rdi + 0], rax                 ; rax = original syscall number
    mov rax, [gs:CPU_SYSCALL_SAVED_RBX]
    mov [rdi + 8], rax
    mov rax, [gs:CPU_SYSCALL_SAVED_RCX]
    mov [rdi + 16], rax
    mov rax, [gs:CPU_SYSCALL_SAVED_RDX]
    mov [rdi + 24], rax
    mov rax, [gs:CPU_SYSCALL_SAVED_RSI]
    mov [rdi + 32], rax
    mov rax, [gs:CPU_SYSCALL_SAVED_RDI]
    mov [rdi + 40], rax
    mov rax, [gs:CPU_SYSCALL_SAVED_RBP]
    mov [rdi + 48], rax
    mov rax, [gs:CPU_SYSCALL_USER_RSP]
    mov [rdi + 56], rax
    mov rax, [gs:CPU_SYSCALL_SAVED_R8]
    mov [rdi + 64], rax
    mov rax, [gs:CPU_SYSCALL_SAVED_R9]
    mov [rdi + 72], rax
    mov rax, [gs:CPU_SYSCALL_SAVED_R10]
    mov [rdi + 80], rax
    mov rax, [gs:CPU_SYSCALL_SAVED_R11]
    mov [rdi + 88], rax
    mov rax, [gs:CPU_SYSCALL_SAVED_R12]
    mov [rdi + 96], rax
    mov rax, [gs:CPU_SYSCALL_SAVED_R13]
    mov [rdi + 104], rax
    mov rax, [gs:CPU_SYSCALL_SAVED_R14]
    mov [rdi + 112], rax
    mov rax, [gs:CPU_SYSCALL_SAVED_R15]
    mov [rdi + 120], rax
    mov rax, [gs:CPU_SYSCALL_SAVED_RCX]
    sub rax, 2                         ; rip = the 2-byte `syscall` instruction
    mov [rdi + 128], rax
    mov rax, [gs:CPU_SYSCALL_SAVED_R11]
    mov [rdi + 136], rax
    ret

global syscall_escape_user_fault
syscall_escape_user_fault:
    cmp qword [gs:CPU_SYSCALL_SMOKE_ACTIVE], 1
    jne .halt
    mov qword [gs:CPU_SYSCALL_SMOKE_ACTIVE], 0
    mov rsp, [gs:CPU_SYSCALL_SMOKE_KERNEL_RSP]
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbp
    pop rbx
    sti
    ret
.halt:
    cli
    hlt
    jmp .halt

global run_user_syscall_smoke
run_user_syscall_smoke:
    ; Called from Rust as an extern "C" function. The user payload can clobber
    ; SysV callee-saved registers before returning through the syscall
    ; trampoline, so preserve the kernel caller's callee-saved state here.
    push rbx
    push rbp
    push r12
    push r13
    push r14
    push r15
    mov [gs:CPU_SYSCALL_SMOKE_KERNEL_RSP], rsp
    mov qword [gs:CPU_SYSCALL_SMOKE_ACTIVE], 1
    mov rax, rdi
    mov rbx, rsi

    push qword USER_DATA_SELECTOR
    push rbx
    pushfq
    pop rcx
    or rcx, 0x200
    push rcx
    push qword USER_CODE_SELECTOR
    push rax
    xor rax, rax
    xor rcx, rcx
    xor rdx, rdx
    xor rsi, rsi
    xor rdi, rdi
    xor r8, r8
    xor r9, r9
    xor r10, r10
    xor r11, r11
    swapgs
    iretq

global run_user_process
run_user_process:
    ; Rust extern "C":
    ;   rdi=entry, rsi=user_stack, rdx=argc, rcx=argv, r8=envp
    ; Userspace entry ABI:
    ;   stack points at the argc/argv/envp block,
    ;   rdi=argc, rsi=argv, rdx=envp for tiny no-libc programs.
    push rbx
    push rbp
    push r12
    push r13
    push r14
    push r15
    mov [gs:CPU_SYSCALL_SMOKE_KERNEL_RSP], rsp
    mov qword [gs:CPU_SYSCALL_SMOKE_ACTIVE], 1
    mov rax, rdi
    mov rbx, rsi
    mov r12, rdx
    mov r13, rcx
    mov r14, r8

    push qword USER_DATA_SELECTOR
    push rbx
    pushfq
    pop rcx
    or rcx, 0x200
    push rcx
    push qword USER_CODE_SELECTOR
    push rax
    xor rax, rax
    xor rcx, rcx
    mov rdi, r12
    mov rsi, r13
    mov rdx, r14
    xor r8, r8
    xor r9, r9
    xor r10, r10
    xor r11, r11
    xor r12, r12
    xor r13, r13
    xor r14, r14
    swapgs
    iretq

global run_user_context
run_user_context:
    ; Rust extern "C":
    ;   rdi = *const CpuContext
    push rbx
    push rbp
    push r12
    push r13
    push r14
    push r15
    mov [gs:CPU_SYSCALL_SMOKE_KERNEL_RSP], rsp
    mov qword [gs:CPU_SYSCALL_SMOKE_ACTIVE], 1
    mov rbx, rdi
    mov rax, [gs:CPU_USER_FPU_STATE]
    test rax, rax
    jz .no_user_fpu
    fxrstor [rax]
.no_user_fpu:

    mov rax, [rbx + 0]
    mov rcx, [rbx + 16]
    mov rdx, [rbx + 24]
    mov rsi, [rbx + 32]
    mov rdi, [rbx + 40]
    mov rbp, [rbx + 48]
    mov r8, [rbx + 64]
    mov r9, [rbx + 72]
    mov r10, [rbx + 80]
    mov r11, [rbx + 88]
    mov r12, [rbx + 96]
    mov r13, [rbx + 104]
    mov r14, [rbx + 112]
    mov r15, [rbx + 120]

    push qword USER_DATA_SELECTOR
    push qword [rbx + 56]
    push qword [rbx + 136]
    push qword USER_CODE_SELECTOR
    push qword [rbx + 128]
    mov rbx, [rbx + 8]
    swapgs
    iretq

section .rodata
align 8
syscall_smoke_return_magic:
    dq 0x0051595343414C4C
