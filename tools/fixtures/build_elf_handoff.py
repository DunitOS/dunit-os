#!/usr/bin/env python3
"""Build the opt-in ELF handoff smoke overlay with the host assembler and ld."""
from pathlib import Path
import argparse
import subprocess
from tempfile import TemporaryDirectory


DIRECTORY = "tests/elf-handoff"
MAIN = r"""
#include <dunit/syscall.h>
.section .interp,"a"
.asciz "/tests/elf-handoff/interpreter"
.text
.global _start
_start:
    mov $37, %edi
    cmp $0x534d4f4b, %r12d
    je .Lexit
    mov $1, %edi
.Lexit:
    mov $SYS_EXIT, %eax
    syscall
    ud2
.section .note.GNU-stack,""
"""

MAIN_LINK = """
ENTRY(_start)
PHDRS {
    interp PT_INTERP;
    image PT_LOAD FILEHDR PHDRS FLAGS(5);
}
SECTIONS {
    . = 0x400000 + SIZEOF_HEADERS;
    .interp : { *(.interp) } :image :interp
    .text : { *(.text) } :image
}
"""

INTERPRETER = r"""
#include <dunit/syscall.h>
.text
.global _start
.hidden __ehdr_start
_start:
    cmpq $1, (%rsp)
    jne .Lfail
    cmpq $0, 16(%rsp)
    jne .Lfail
    cmpq $0, 24(%rsp)
    jne .Lfail

    lea 32(%rsp), %rsi
    xor %r8d, %r8d
    xor %r9d, %r9d
    xor %r10d, %r10d
    mov $16, %ecx
.Laux:
    mov (%rsi), %rax
    mov 8(%rsi), %rdx
    test %rax, %rax
    je .Lcheck
    cmp $3, %rax
    cmove %rdx, %r8
    cmp $7, %rax
    cmove %rdx, %r9
    cmp $9, %rax
    cmove %rdx, %r10
    add $16, %rsi
    loop .Laux
    jmp .Lfail
.Lcheck:
    lea __ehdr_start(%rip), %rdx
    cmp %rdx, %r9
    jne .Lfail
    cmp $0x400040, %r8
    jne .Lfail
    cmpl $3, (%r8)
    jne .Lfail
    cmpl $1, 56(%r8)
    jne .Lfail
    cmp %r10, -40(%r8)
    jne .Lfail
    mov $0x534d4f4b, %r12d
    jmp *%r10
.Lfail:
    mov $2, %edi
    mov $SYS_EXIT, %eax
    syscall
    ud2
.section .note.GNU-stack,""
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("overlay", type=Path)
    overlay = parser.parse_args().overlay / DIRECTORY
    overlay.mkdir(parents=True, exist_ok=True)
    include = Path(__file__).resolve().parents[2] / "abi/include"
    with TemporaryDirectory() as work:
        temp = Path(work)
        for name, source in (("main", MAIN), ("interpreter", INTERPRETER)):
            assembly = temp / f"{name}.S"
            assembly.write_text(source)
            subprocess.run(
                ["cc", "-m64", "-I", str(include), "-c", assembly, "-o", temp / f"{name}.o"],
                check=True,
            )
        script = temp / "main.ld"
        script.write_text(MAIN_LINK)
        subprocess.run(
            ["ld", "-T", script, "-o", overlay / "main", temp / "main.o"], check=True
        )
        subprocess.run(
            ["ld", "-shared", "-e", "_start", "-o", overlay / "interpreter", temp / "interpreter.o"],
            check=True,
        )


if __name__ == "__main__":
    main()
