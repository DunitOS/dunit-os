/* Disabled input, focus and rendering are implemented by the Rust SDK. */
#include <dunit.h>
struct Model { unsigned enabled, count, checked, transitions; };
static DunitUiNodeV1 node(uint32_t kind, const char *key, const char *text, uint64_t message)
{
    DunitUiNodeV1 value = {kind, 0, (const uint8_t *)key, strlen(key),
        (const uint8_t *)text, strlen(text), message};
    return value;
}
static int32_t build(void *user, DunitUiNodeV1 *nodes, uint32_t capacity)
{
    struct Model *model = user;
    if (capacity < 6) return -DUNIT_EINVAL;
    uint32_t flags = model->enabled ? 0 : DUNIT_UI_DISABLED;
    nodes[0] = node(DUNIT_UI_COLUMN, "root", "", 0);
    nodes[0].parent = DUNIT_UI_ROOT;
    nodes[1] = node(DUNIT_UI_TEXT, "title", "C disabled controls", 0);
    nodes[2] = node(DUNIT_UI_BUTTON | flags, "action", "Action", 1);
    nodes[3] = node((model->checked ? DUNIT_UI_CHECKBOX_CHECKED : DUNIT_UI_CHECKBOX) | flags,
        "check", "Checkbox", 2);
    nodes[4] = node(model->enabled ? DUNIT_UI_TOGGLE_BUTTON_CHECKED : DUNIT_UI_TOGGLE_BUTTON,
        "enable", "Enable controls", 3);
    nodes[5] = node(DUNIT_UI_BUTTON, "quit", "Quit", 8);
    return 6;
}
static int32_t update(void *user, uint64_t message)
{
    struct Model *model = user;
    if (message == 8) return 1;
    if (message == 3) {
        model->enabled ^= 1; model->transitions++;
        printf("[C-DISABLED] enabled %u\n", model->enabled);
    } else if (!model->enabled) return -DUNIT_EINVAL;
    else if (message == 1) { model->count++; puts("[C-DISABLED] action"); }
    else if (message == 2) { model->checked ^= 1; puts("[C-DISABLED] checked"); }
    else return -DUNIT_EINVAL;
    return 0;
}
int main(int argc, char **argv, char **envp)
{
    setvbuf(stdout, NULL, _IONBF, 0);
    struct Model model = {0};
    DunitUiCallbacksV1 callbacks = {DUNIT_UI_ABI_VERSION, sizeof callbacks, &model, build, update};
    int result = dunit_ui_run_v1(argc, (const char *const *)argv,
        (const char *const *)envp, 400, 240, &callbacks);
    if (result || model.enabled || model.count != 1 || model.checked != 1 || model.transitions != 2) return 1;
    puts("[C-DISABLED] ok");
    return 0;
}
