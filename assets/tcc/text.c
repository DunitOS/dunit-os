/* All editing, selection, history and painting are the ordinary Rust SDK. */
#include <dunit.h>
_Static_assert(sizeof(DunitUiInputV1) == 56, "input configuration ABI");
_Static_assert(sizeof(DunitUiTextV1) == 24, "text callbacks ABI");
enum { NAME_MAX_CHARS = 8, FIELD_MAX_CHARS = 128 };
/* A Unicode scalar needs at most four UTF-8 bytes, plus the C terminator. */
struct Model { char name[4*NAME_MAX_CHARS+1], password[4*FIELD_MAX_CHARS+1], notes[4*FIELD_MAX_CHARS+1], status[96]; unsigned accepted, declined, submits, resets; int invalid; };
static DunitUiNodeV1 node(uint32_t kind, uint32_t parent, const char *key, const char *value, uint64_t id)
{
    DunitUiNodeV1 n = {kind, parent, (const uint8_t *)key, strlen(key), (const uint8_t *)value, strlen(value), id}; return n;
}
static int32_t build(void *user, DunitUiNodeV1 *n, uint32_t capacity)
{
    struct Model *m = user; if (capacity < 11) return -DUNIT_EINVAL;
    snprintf(m->status, sizeof m->status, "Accepted %u / declined %u / submits %u", m->accepted, m->declined, m->submits);
    n[0] = node(DUNIT_UI_COLUMN, DUNIT_UI_ROOT, "root", "", 0);
    n[1] = node(DUNIT_UI_TEXT, 0, "title", "C SDK text editor", 0);
    n[2] = node(DUNIT_UI_INPUT, 0, "name", m->name, 101);
    n[3] = node(DUNIT_UI_INPUT, 0, "password", m->password, 102);
    n[4] = node(DUNIT_UI_TEXTAREA, 0, "notes", m->notes, UINT64_MAX);
    n[5] = node(DUNIT_UI_INPUT, 0, "readonly", "read only", 104);
    n[6] = node(DUNIT_UI_INPUT | DUNIT_UI_DISABLED, 0, "disabled", "unavailable", 105);
    n[7] = node(DUNIT_UI_ROW, 0, "actions", "", 0);
    n[8] = node(DUNIT_UI_BUTTON, 7, "reset", "Reset", 10);
    n[9] = node(DUNIT_UI_BUTTON, 7, "quit", "Quit", 8);
    n[10] = node(DUNIT_UI_TEXT, 0, "status", m->status, 0); return 11;
}
static int32_t configure(void *user, uint32_t index, DunitUiInputV1 *input)
{
    struct Model *m = user;
    static const char *labels[] = {"Name", "Password", "Notes", "Read only", "Disabled"};
    if (index < 2 || index > 6) return -DUNIT_EINVAL;
    const char *label = labels[index-2]; input->label = (const uint8_t *)label; input->label_len = strlen(label);
    input->placeholder = (const uint8_t *)label; input->placeholder_len = strlen(label);
    input->max_chars = index == 2 ? NAME_MAX_CHARS : FIELD_MAX_CHARS;
    if (index == 2) { input->flags = DUNIT_UI_INPUT_SUBMIT; input->submit = 201; }
    if (index == 3) input->flags = DUNIT_UI_INPUT_PASSWORD;
    if (index == 4) input->flags = DUNIT_UI_INPUT_SOFT_WRAP;
    if (index == 5) input->flags = DUNIT_UI_INPUT_READ_ONLY;
    if (m->invalid) input->flags = 0x80000000u;
    return 0;
}
static int32_t changed(void *user, uint64_t id, const uint8_t *value, size_t length)
{
    struct Model *m = user; char *out; size_t capacity;
    if (id == 101 && memchr(value, '!', length)) { m->declined++; puts("[C-TEXT] edit declined"); return 0; }
    if (id == 101) { out = m->name; capacity = sizeof m->name; }
    else if (id == 102) { out = m->password; capacity = sizeof m->password; }
    else if (id == UINT64_MAX) { out = m->notes; capacity = sizeof m->notes; }
    else return -DUNIT_EINVAL;
    if (length >= capacity) return -DUNIT_EINVAL;
    memcpy(out, value, length); out[length] = 0; m->accepted++;
    if (id == 101 && length == 8) puts("[C-TEXT] name limit reached");
    if (id == 102 && length == 3) puts("[C-TEXT] password edited");
    if (id == UINT64_MAX && !strcmp(out, "line1\nline2")) puts("[C-TEXT] multiline edited");
    return 0;
}
static int32_t update(void *user, uint64_t id)
{
    struct Model *m = user;
    if (id == 8) return 1;
    if (id == 201) { m->submits++; puts("[C-TEXT] submitted"); return 0; }
    if (id != 10 || strcmp(m->name, "alexande") || strcmp(m->password, "xyz") || strcmp(m->notes, "line1\nline2") || m->accepted != 22 || m->declined != 1 || m->submits != 1) return -DUNIT_EINVAL;
    strcpy(m->name, "guest"); strcpy(m->password, "secret"); strcpy(m->notes, "hello"); m->resets++;
    puts("[C-TEXT] model reset"); return 0;
}
int main(int argc, char **argv, char **envp)
{
    setvbuf(stdout, NULL, _IONBF, 0);
    FILE *file = fopen("/assets/tcc/text.dss", "rb"); if (!file) return 1;
    char sheet[8192]; size_t length = fread(sheet, 1, sizeof sheet, file);
    if (ferror(file) || !feof(file)) { fclose(file); return 2; } fclose(file);
    struct Model m = {"guest", "secret", "hello", "", 0};
    DunitUiCallbacksV1 callbacks = {DUNIT_UI_ABI_VERSION, sizeof callbacks, &m, build, update};
    DunitUiStyleV1 style = {DUNIT_UI_ABI_VERSION, sizeof style, (const uint8_t *)sheet, length, NULL};
    DunitUiTextV1 text = {DUNIT_UI_ABI_VERSION, sizeof text, configure, changed};
    text.abi_version++;
    if (dunit_ui_run_text_v1(argc, (const char *const *)argv, (const char *const *)envp, 560, 460, &callbacks, &style, &text) != -DUNIT_EINVAL) return 3;
    text.abi_version--; m.invalid = 1;
    if (dunit_ui_run_text_v1(argc, (const char *const *)argv, (const char *const *)envp, 560, 460, &callbacks, &style, &text) != -DUNIT_EINVAL) return 4;
    m.invalid = 0; puts("[C-TEXT] invalid configuration rejected");
    int result = dunit_ui_run_text_v1(argc, (const char *const *)argv, (const char *const *)envp, 560, 460, &callbacks, &style, &text);
    if (result || m.resets != 1 || strcmp(m.name, "guest") || strcmp(m.password, "secret") || strcmp(m.notes, "hello")) return 5;
    puts("[C-TEXT] ok"); return 0;
}
