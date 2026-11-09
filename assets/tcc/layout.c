/* Classes and DSS feed the same layout/theme pipeline as native Rust apps. */
#include <dunit.h>
_Static_assert(sizeof(DunitUiAppearanceV1) == 16, "appearance ABI");
_Static_assert(sizeof(DunitUiStyleV1) == 32, "style ABI");
struct Model { unsigned wide, warm, selected, updates; int invalid; char summary[64]; };
static const char *labels[] = {"Storage", "Display", "Network", "Theme"};
static DunitUiNodeV1 node(uint32_t kind, uint32_t parent, const char *key, const char *text, uint64_t message)
{
    DunitUiNodeV1 value = {kind, parent, (const uint8_t *)key, strlen(key), (const uint8_t *)text, strlen(text), message};
    return value;
}
static int32_t build(void *user, DunitUiNodeV1 *nodes, uint32_t capacity)
{
    struct Model *m = user;
    if (capacity < 15) return -DUNIT_EINVAL;
    snprintf(m->summary, sizeof m->summary, "Selected: %s", labels[m->selected]);
    nodes[0] = node(DUNIT_UI_COLUMN, DUNIT_UI_ROOT, "root", "", 0);
    nodes[1] = node(DUNIT_UI_TEXT, 0, "title", "C SDK layout lab", 0);
    nodes[2] = node(DUNIT_UI_STACK, 0, "hero", "", 0);
    nodes[3] = node(DUNIT_UI_COLUMN, 2, "backdrop", "", 0);
    nodes[4] = node(DUNIT_UI_TEXT, 2, "summary", m->summary, 0);
    nodes[5] = node(DUNIT_UI_ROW, 0, "controls", "", 0);
    nodes[6] = node(DUNIT_UI_BUTTON, 5, "layout", "Layout", 1);
    nodes[7] = node(DUNIT_UI_BUTTON, 5, "palette", "Palette", 2);
    nodes[8] = node(DUNIT_UI_BUTTON, 5, "quit", "Quit", 8);
    nodes[9] = node(DUNIT_UI_GRID, 0, "cards", "", 0);
    for (unsigned i = 0; i < 4; i++) nodes[10+i] = node(DUNIT_UI_BUTTON, 9, labels[i], labels[i], 10+i);
    nodes[14] = node(DUNIT_UI_TEXT, 0, "hint", "Tab / Space: layout, palette, cards", 0);
    return 15;
}
static int32_t decorate(void *user, uint32_t index, DunitUiAppearanceV1 *appearance)
{
    struct Model *m = user;
    const char *classes = "";
    if (m->invalid) classes = "bad!";
    else if (index == 0) classes = m->warm ? "page warm" : "page";
    else if (index == 1) classes = "heading";
    else if (index == 2) classes = "hero";
    else if (index == 3) classes = "backdrop";
    else if (index == 4) classes = "summary";
    else if (index == 5) classes = "controls";
    else if (index <= 8) classes = "control";
    else if (index == 9) classes = m->wide ? "cards wide" : "cards";
    else if (index <= 13) classes = index-10 == m->selected ? (m->warm ? "card selected warm" : "card selected") : "card";
    else classes = "hint";
    appearance->classes = (const uint8_t *)classes;
    appearance->classes_len = strlen(classes);
    return 0;
}
static int32_t update(void *user, uint64_t message)
{
    struct Model *m = user;
    if (message == 8) return 1;
    if (message == 1) { m->wide ^= 1; puts("[C-STYLE] layout changed"); }
    else if (message == 2) { m->warm ^= 1; puts("[C-STYLE] palette changed"); }
    else if (message >= 10 && message <= 13) { m->selected = message-10; puts("[C-STYLE] selection changed"); }
    else return -DUNIT_EINVAL;
    m->updates++;
    return 0;
}
int main(int argc, char **argv, char **envp)
{
    setvbuf(stdout, NULL, _IONBF, 0);
    FILE *file = fopen("/assets/tcc/layout.dss", "rb");
    if (!file) return 1;
    char sheet[8192]; size_t length = fread(sheet, 1, sizeof sheet, file);
    if (ferror(file) || !feof(file)) { fclose(file); return 2; }
    fclose(file);
    struct Model model = {0};
    DunitUiCallbacksV1 callbacks = {DUNIT_UI_ABI_VERSION, sizeof callbacks, &model, build, update};
    DunitUiStyleV1 style = {DUNIT_UI_ABI_VERSION, sizeof style, (const uint8_t *)sheet, length, decorate};
    style.abi_version++;
    if (dunit_ui_run_styled_v1(argc, (const char *const *)argv, (const char *const *)envp, 560, 360, &callbacks, &style) != -DUNIT_EINVAL) return 3;
    style.abi_version--; model.invalid = 1;
    if (dunit_ui_run_styled_v1(argc, (const char *const *)argv, (const char *const *)envp, 560, 360, &callbacks, &style) != -DUNIT_EINVAL) return 4;
    model.invalid = 0; puts("[C-STYLE] invalid configuration rejected");
    int result = dunit_ui_run_styled_v1(argc, (const char *const *)argv, (const char *const *)envp, 560, 360, &callbacks, &style);
    if (result || !model.wide || !model.warm || model.selected != 1 || model.updates != 3) return 5;
    if (dunit_ui_run_v1(argc, (const char *const *)argv, (const char *const *)envp, 560, 360, &callbacks) != -DUNIT_EPIPE) return 6;
    puts("[C-STYLE] shared session enforced");
    puts("[C-STYLE] ok"); return 0;
}
