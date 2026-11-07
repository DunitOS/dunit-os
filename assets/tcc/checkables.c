/* Controlled state belongs to C; widgets and input belong to the Rust SDK. */
#include <dunit.h>
struct Model { unsigned bits, changes; };
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
    nodes[0] = node(DUNIT_UI_COLUMN, "root", "", 0);
    nodes[0].parent = DUNIT_UI_ROOT;
    nodes[1] = node(DUNIT_UI_TEXT, "title", "C controlled widgets", 0);
    nodes[2] = node(model->bits & 1 ? DUNIT_UI_CHECKBOX_CHECKED : DUNIT_UI_CHECKBOX,
        "checkbox", "Checkbox", 1);
    nodes[3] = node(model->bits & 2 ? DUNIT_UI_SWITCH_CHECKED : DUNIT_UI_SWITCH,
        "switch", "Switch", 2);
    nodes[4] = node(model->bits & 4 ? DUNIT_UI_TOGGLE_BUTTON_CHECKED : DUNIT_UI_TOGGLE_BUTTON,
        "toggle", "Toggle button", 4);
    nodes[5] = node(DUNIT_UI_BUTTON, "quit", "Quit", 8);
    return 6;
}
static int32_t update(void *user, uint64_t message)
{
    struct Model *model = user;
    if (message == 8) return 1;
    if (message != 1 && message != 2 && message != 4) return -DUNIT_EINVAL;
    model->bits ^= (unsigned)message;
    model->changes++;
    printf("[C-CHECKABLE] change %u bits %u\n", model->changes, model->bits);
    return 0;
}
int main(int argc, char **argv, char **envp)
{
    setvbuf(stdout, NULL, _IONBF, 0);
    struct Model model = {0};
    DunitUiCallbacksV1 callbacks = {DUNIT_UI_ABI_VERSION, sizeof callbacks, &model, build, update};
    int result = dunit_ui_run_v1(argc, (const char *const *)argv,
        (const char *const *)envp, 400, 240, &callbacks);
    if (result || model.bits || model.changes != 6) return 1;
    puts("[C-CHECKABLE] ok");
    return 0;
}
