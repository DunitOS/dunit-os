/* Group keys and selection are C data; all radio behavior is the Rust SDK. */
#include <dunit.h>
struct Model { unsigned selected[2], changes; int invalid; };
static DunitUiNodeV1 node(uint32_t kind, uint32_t parent, const char *key,
                         const char *text, uint64_t message)
{
    DunitUiNodeV1 value = {kind, parent, (const uint8_t *)key, strlen(key),
        (const uint8_t *)text, strlen(text), message};
    return value;
}
static int32_t build(void *user, DunitUiNodeV1 *nodes, uint32_t capacity)
{
    struct Model *model = user;
    if (capacity < 9) return -DUNIT_EINVAL;
    nodes[0] = node(DUNIT_UI_COLUMN, DUNIT_UI_ROOT, "root", "", 0);
    nodes[1] = node(DUNIT_UI_TEXT, 0, "title", "C radio groups", 0);
    nodes[2] = node(DUNIT_UI_RADIO_GROUP_COLUMN, 0, "main", "", 0);
    nodes[3] = node(model->invalid || model->selected[0] == 0 ? DUNIT_UI_RADIO_CHECKED : DUNIT_UI_RADIO, 2, "alpha", "Alpha", 17);
    nodes[4] = node(model->invalid || model->selected[0] == 1 ? DUNIT_UI_RADIO_CHECKED : DUNIT_UI_RADIO, 2, "beta", "Beta", UINT64_MAX);
    nodes[5] = node(DUNIT_UI_RADIO_GROUP_ROW, 0, "mode", "", 0);
    nodes[6] = node(model->selected[1] == 0 ? DUNIT_UI_RADIO_CHECKED : DUNIT_UI_RADIO, 5, "light", "Light", 29);
    nodes[7] = node(model->selected[1] == 1 ? DUNIT_UI_RADIO_CHECKED : DUNIT_UI_RADIO, 5, "dark", "Dark", 31);
    nodes[8] = node(DUNIT_UI_BUTTON, 0, "quit", "Quit", 8);
    return 9;
}
static int32_t update(void *user, uint64_t message)
{
    struct Model *model = user;
    if (message == 8) return 1;
    if (message == 17 || message == UINT64_MAX) model->selected[0] = message == UINT64_MAX;
    else if (message == 29 || message == 31) model->selected[1] = message == 31;
    else return -DUNIT_EINVAL;
    printf("[C-RADIO] change %u selected %u,%u\n", ++model->changes, model->selected[0], model->selected[1]);
    return 0;
}
int main(int argc, char **argv, char **envp)
{
    setvbuf(stdout, NULL, _IONBF, 0);
    struct Model model = {{0, 0}, 0, 1};
    DunitUiCallbacksV1 callbacks = {DUNIT_UI_ABI_VERSION, sizeof callbacks, &model, build, update};
    if (dunit_ui_run_v1(argc, (const char *const *)argv, (const char *const *)envp, 400, 240, &callbacks) != -DUNIT_EINVAL) return 1;
    puts("[C-RADIO] invalid group rejected");
    model.invalid = 0;
    int result = dunit_ui_run_v1(argc, (const char *const *)argv, (const char *const *)envp, 400, 240, &callbacks);
    if (result || model.selected[0] != 1 || model.selected[1] != 1 || model.changes != 6) return 2;
    puts("[C-RADIO] ok");
    return 0;
}
