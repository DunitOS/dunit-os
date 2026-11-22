/* A bounded, fixture-only loader for one ET_DYN dependency. */
#include <dunit/syscall.h>
#include <elf.h>
#include <stddef.h>
#include <stdint.h>

enum {
    PAGE = 4096, MAX_IMAGE = 64 * 1024, MAX_PHDR = 16,
    READ = 1, WRITE = 2, EXEC = 4, PRIVATE = 2, ANONYMOUS = 32,
};
static const char dependency[] = "libhandoff.so";
static const char path[] = "/tests/elf-handoff/libhandoff.so";
extern const char __ehdr_start[] __attribute__((visibility("hidden")));
extern _Noreturn void enter_main(uintptr_t *stack, uintptr_t entry)
    __attribute__((visibility("hidden")));

static long native(long number, long a, long b, long c, long d, long e, long f)
{
    register long r10 __asm__("r10") = d;
    register long r8 __asm__("r8") = e;
    register long r9 __asm__("r9") = f;
    long result;
    __asm__ volatile ("syscall" : "=a"(result)
        : "a"(number), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9)
        : "rcx", "r11", "memory");
    return result;
}

static _Noreturn void fail(void)
{
    native(SYS_EXIT, 2, 0, 0, 0, 0, 0);
    __builtin_trap();
}

static int fits(size_t start, size_t size, size_t total)
{
    return start <= total && size <= total - start;
}

struct image {
    uintptr_t base;
    const Elf64_Phdr *phdr;
    size_t phnum;
};

static void *address(struct image image, size_t offset, size_t size, unsigned flags)
{
    for (size_t i = 0; i < image.phnum; ++i) {
        const Elf64_Phdr *p = &image.phdr[i];
        if (p->p_type == PT_LOAD && (p->p_flags & flags) == flags &&
            offset >= p->p_vaddr && fits(offset - p->p_vaddr, size, p->p_memsz))
            return (void *)(image.base + offset);
    }
    fail();
}

static const Elf64_Dyn *dynamic(struct image image, size_t *count)
{
    for (size_t i = 0; i < image.phnum; ++i) {
        const Elf64_Phdr *p = &image.phdr[i];
        if (p->p_type == PT_DYNAMIC) {
            *count = p->p_filesz / sizeof(Elf64_Dyn);
            return address(image, p->p_vaddr, p->p_filesz, PF_R);
        }
    }
    fail();
}

static size_t tag(struct image image, long id)
{
    size_t count;
    const Elf64_Dyn *entries = dynamic(image, &count);
    for (size_t i = 0; i < count && entries[i].d_tag != DT_NULL; ++i)
        if (entries[i].d_tag == id)
            return entries[i].d_un.d_val;
    fail();
}

static const char *name(struct image image, size_t offset)
{
    size_t size = tag(image, DT_STRSZ);
    if (offset >= size)
        fail();
    const char *strings = address(image, tag(image, DT_STRTAB), size, PF_R);
    for (size_t i = offset; i < size; ++i)
        if (!strings[i])
            return strings + offset;
    fail();
}

static int equal(const char *a, const char *b)
{
    while (*a && *a == *b) {
        ++a;
        ++b;
    }
    return *a == *b;
}

static size_t symbol_count(struct image image)
{
    const uint32_t *hash = address(image, tag(image, DT_HASH), 2 * sizeof(*hash), PF_R);
    if (hash[1] > 128)
        fail();
    return hash[1];
}

static const Elf64_Sym *symbols(struct image image, size_t count)
{
    if (tag(image, DT_SYMENT) != sizeof(Elf64_Sym))
        fail();
    return address(image, tag(image, DT_SYMTAB), count * sizeof(Elf64_Sym), PF_R);
}

static struct image map_dependency(const unsigned char **file, size_t *file_size)
{
    struct { uint32_t kind, reserved; size_t size; } stat;
    long fd = native(SYS_OPEN, (long)path, sizeof(path) - 1, READ, 0, 0, 0);
    if (fd < 0 || native(SYS_FSTAT, fd, (long)&stat, 0, 0, 0, 0) ||
        stat.size < sizeof(Elf64_Ehdr) || stat.size > MAX_IMAGE)
        fail();
    long view = native(SYS_MMAP, 0, stat.size, READ, PRIVATE, fd, 0);
    if (view < 0 || native(SYS_CLOSE, fd, 0, 0, 0, 0, 0))
        fail();
    const Elf64_Ehdr *header = (const void *)view;
    if (header->e_ident[EI_MAG0] != ELFMAG0 || header->e_ident[EI_MAG1] != ELFMAG1 ||
        header->e_ident[EI_MAG2] != ELFMAG2 || header->e_ident[EI_MAG3] != ELFMAG3 ||
        header->e_ident[EI_CLASS] != ELFCLASS64 || header->e_ident[EI_DATA] != ELFDATA2LSB ||
        header->e_type != ET_DYN || header->e_machine != EM_X86_64 ||
        header->e_phentsize != sizeof(Elf64_Phdr) || header->e_phnum > MAX_PHDR ||
        !fits(header->e_phoff, header->e_phnum * sizeof(Elf64_Phdr), stat.size))
        fail();
    *file = (const void *)view;
    *file_size = stat.size;
    struct image dso = {0, (const void *)(*file + header->e_phoff), header->e_phnum};
    size_t end = 0;
    for (size_t i = 0; i < dso.phnum; ++i) {
        const Elf64_Phdr *p = &dso.phdr[i];
        if (p->p_type != PT_LOAD || !p->p_memsz)
            continue;
        size_t start = p->p_vaddr & ~(PAGE - 1);
        if (start < end || (p->p_flags & (PF_W | PF_X)) == (PF_W | PF_X) ||
            p->p_filesz > p->p_memsz || !fits(p->p_offset, p->p_filesz, stat.size) ||
            !fits(p->p_vaddr, p->p_memsz, MAX_IMAGE))
            fail();
        end = (p->p_vaddr + p->p_memsz + PAGE - 1) & ~(PAGE - 1);
    }
    if (!end)
        fail();
    long mapped = native(SYS_MMAP, 0, end, READ | WRITE, PRIVATE | ANONYMOUS, -1, 0);
    if (mapped < 0)
        fail();
    dso.base = mapped;
    for (size_t i = 0; i < dso.phnum; ++i) {
        const Elf64_Phdr *p = &dso.phdr[i];
        if (p->p_type != PT_LOAD || !p->p_memsz)
            continue;
        unsigned char *dest = (void *)(dso.base + p->p_vaddr);
        for (size_t j = 0; j < p->p_filesz; ++j)
            dest[j] = (*file)[p->p_offset + j];
    }
    return dso;
}

static void relocate_dependency(struct image dso)
{
    if (tag(dso, DT_RELAENT) != sizeof(Elf64_Rela) ||
        tag(dso, DT_RELASZ) != sizeof(Elf64_Rela))
        fail();
    const Elf64_Rela *rel = address(dso, tag(dso, DT_RELA), sizeof(*rel), PF_R);
    if (rel->r_info != ELF64_R_INFO(0, R_X86_64_RELATIVE))
        fail();
    uintptr_t *slot = address(dso, rel->r_offset, sizeof(*slot), PF_W);
    *slot = (uintptr_t)address(dso, rel->r_addend, 1, PF_R);

    for (size_t i = 0; i < dso.phnum; ++i) {
        const Elf64_Phdr *p = &dso.phdr[i];
        if (p->p_type != PT_LOAD || !p->p_memsz)
            continue;
        size_t start = p->p_vaddr & ~(PAGE - 1);
        size_t length = ((p->p_vaddr + p->p_memsz + PAGE - 1) & ~(PAGE - 1)) - start;
        int prot = (p->p_flags & PF_R ? READ : 0) | (p->p_flags & PF_W ? WRITE : 0) |
            (p->p_flags & PF_X ? EXEC : 0);
        if (native(SYS_MPROTECT, dso.base + start, length, prot, 0, 0, 0))
            fail();
    }
}

static void bind_import(struct image main, struct image dso)
{
    if (tag(main, DT_PLTREL) != DT_RELA || tag(main, DT_PLTRELSZ) != sizeof(Elf64_Rela))
        fail();
    const Elf64_Rela *rel = address(main, tag(main, DT_JMPREL), sizeof(*rel), PF_R);
    size_t index = ELF64_R_SYM(rel->r_info), count = symbol_count(main);
    if (ELF64_R_TYPE(rel->r_info) != R_X86_64_JUMP_SLOT || rel->r_addend || !index || index >= count)
        fail();
    const Elf64_Sym *import = symbols(main, count) + index;
    if (import->st_shndx != SHN_UNDEF)
        fail();
    const char *wanted = name(main, import->st_name);
    count = symbol_count(dso);
    const Elf64_Sym *exports = symbols(dso, count);
    for (size_t i = 1; i < count; ++i) {
        const Elf64_Sym *symbol = exports + i;
        if (symbol->st_shndx != SHN_UNDEF && ELF64_ST_TYPE(symbol->st_info) == STT_FUNC &&
            ELF64_ST_BIND(symbol->st_info) == STB_GLOBAL && equal(wanted, name(dso, symbol->st_name))) {
            uintptr_t *slot = address(main, rel->r_offset, sizeof(*slot), PF_W);
            *slot = (uintptr_t)address(dso, symbol->st_value, symbol->st_size, PF_X);
            return;
        }
    }
    fail();
}

__attribute__((visibility("hidden"))) _Noreturn void dso_handoff(uintptr_t *stack)
{
    if (stack[0] != 1 || stack[2] || stack[3])
        fail();
    uintptr_t phdr = 0, phnum = 0, phent = 0, base = 0, entry = 0;
    for (const uintptr_t *aux = stack + 4; aux < stack + 36 && *aux; aux += 2) {
        switch (aux[0]) {
        case AT_PHDR: phdr = aux[1]; break;
        case AT_PHNUM: phnum = aux[1]; break;
        case AT_PHENT: phent = aux[1]; break;
        case AT_BASE: base = aux[1]; break;
        case AT_ENTRY: entry = aux[1]; break;
        }
    }
    if (!phdr || phnum < 2 || phnum > MAX_PHDR || phent != sizeof(Elf64_Phdr) ||
        base != (uintptr_t)__ehdr_start || !entry)
        fail();
    struct image main = {0, (const Elf64_Phdr *)phdr, phnum};
    address(main, phdr, phnum * sizeof(Elf64_Phdr), PF_R);
    address(main, entry, 1, PF_X);
    if (!equal(name(main, tag(main, DT_NEEDED)), dependency))
        fail();
    const unsigned char *file;
    size_t file_size;
    struct image dso = map_dependency(&file, &file_size);
    relocate_dependency(dso);
    bind_import(main, dso);
    if (native(SYS_MUNMAP, (long)file, (file_size + PAGE - 1) & ~(PAGE - 1), 0, 0, 0, 0))
        fail();
    enter_main(stack, entry);
}
