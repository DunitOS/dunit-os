/* SDK ranges, geometry, draft editing and painting; C owns accepted values. */
#include <dunit.h>
_Static_assert(sizeof(DunitUiNumberV1) == 64, "number ABI");
_Static_assert(sizeof(DunitUiNumbersV1) == 24, "numeric callbacks ABI");
_Static_assert(sizeof(DunitUiAppV1) == 32, "application ABI");
struct Model { int64_t h, v, spin; unsigned accepted, declined, edits, toggles, resets, running; int invalid; char note[65], status[96]; };
static DunitUiNodeV1 node(uint32_t kind, uint32_t parent, const char *key, const char *value, uint64_t id)
{ DunitUiNodeV1 n = {kind, parent, (const uint8_t *)key, strlen(key), (const uint8_t *)value, strlen(value), id}; return n; }
static int32_t build(void *user, DunitUiNodeV1 *n, uint32_t capacity)
{
    struct Model *m = user; if (capacity < 17) return -DUNIT_EINVAL;
    snprintf(m->status, sizeof m->status, "H %lld / V %lld / Spin %lld", (long long)m->h, (long long)m->v, (long long)m->spin);
    n[0]=node(DUNIT_UI_COLUMN,DUNIT_UI_ROOT,"root","",0); n[1]=node(DUNIT_UI_TEXT,0,"title","C SDK numeric controls",0);
    n[2]=node(DUNIT_UI_SLIDER,0,"horizontal","",101); n[3]=node(DUNIT_UI_ROW,0,"body","",0);
    n[4]=node(DUNIT_UI_VERTICAL_SLIDER,3,"vertical","",UINT64_MAX); n[5]=node(DUNIT_UI_COLUMN,3,"values","",0);
    n[6]=node(DUNIT_UI_SPINBOX,5,"spin","",102); n[7]=node(DUNIT_UI_PROGRESS,5,"progress","",0); n[8]=node(DUNIT_UI_BUSY,5,"busy","",0);
    n[9]=node(DUNIT_UI_SPINBOX,5,"readonly","",103); n[10]=node(DUNIT_UI_SLIDER|DUNIT_UI_DISABLED,0,"disabled","",104);
    n[11]=node(DUNIT_UI_INPUT,0,"note",m->note,201); n[12]=node(DUNIT_UI_ROW,0,"actions","",0);
    n[13]=node(DUNIT_UI_BUTTON,12,"toggle","Busy",1); n[14]=node(DUNIT_UI_BUTTON,12,"reset","Reset",2); n[15]=node(DUNIT_UI_BUTTON,12,"quit","Quit",8);
    n[16]=node(DUNIT_UI_TEXT,0,"status",m->status,0); return 17;
}
static int32_t configure(void *user, uint32_t index, DunitUiNumberV1 *n)
{
    struct Model *m=user; n->min=-50; n->max=50; n->step=m->invalid?0:5; n->page_step=10;
    if(index==2||index==7)n->value=m->h; else if(index==4)n->value=m->v; else if(index==6)n->value=m->spin;
    else if(index==9)n->flags=DUNIT_UI_NUMBER_READ_ONLY;
    else if(index==8)n->flags=m->running?DUNIT_UI_NUMBER_RUNNING:0;
    else if(index!=10)return -DUNIT_EINVAL;
    return 0;
}
static int32_t changed(void *user,uint64_t id,int64_t value)
{
    struct Model *m=user;
    if(id==101&&value==45){m->declined++;puts("[C-NUMBER] value declined");return 0;}
    if(id==101){m->h=value;if(value==50)puts("[C-NUMBER] horizontal endpoint");}
    else if(id==UINT64_MAX){m->v=value;if(value==15)puts("[C-NUMBER] vertical page");}
    else if(id==102){m->spin=value;if(value==-15)puts("[C-NUMBER] signed draft committed");}
    else return -DUNIT_EINVAL;
    m->accepted++;return 0;
}
static int32_t input(void *user,uint32_t index,DunitUiInputV1 *n)
{ (void)user;if(index!=11)return -DUNIT_EINVAL;n->max_chars=16;return 0; }
static int32_t edited(void *user,uint64_t id,const uint8_t *value,size_t length)
{
    struct Model *m=user;if(id!=201||length>=sizeof m->note)return -DUNIT_EINVAL;
    memcpy(m->note,value,length);m->note[length]=0;m->edits++;
    if(!strcmp(m->note,"ok"))puts("[C-NUMBER] text coexistence");return 0;
}
static int32_t update(void *user,uint64_t id)
{
    struct Model *m=user;if(id==8)return 1;
    if(id==1){m->running^=1;m->toggles++;puts("[C-NUMBER] busy changed");return 0;}
    if(id!=2||m->h!=50||m->v!=15||m->spin!=-15||m->accepted!=6||m->declined!=1||m->edits!=2||m->toggles!=1||strcmp(m->note,"ok"))return -DUNIT_EINVAL;
    m->h=m->v=m->spin=0;m->running=0;strcpy(m->note,"mixed");m->resets++;puts("[C-NUMBER] reset");return 0;
}
int main(int argc,char **argv,char **envp)
{
    setvbuf(stdout,NULL,_IONBF,0);FILE *f=fopen("/assets/tcc/numbers.dss","rb");if(!f)return 1;
    char sheet[8192];size_t len=fread(sheet,1,sizeof sheet,f);if(ferror(f)||!feof(f)){fclose(f);return 2;}fclose(f);
    struct Model m={0};strcpy(m.note,"mixed");
    DunitUiCallbacksV1 callbacks={DUNIT_UI_ABI_VERSION,sizeof callbacks,&m,build,update};
    DunitUiStyleV1 style={DUNIT_UI_ABI_VERSION,sizeof style,(const uint8_t *)sheet,len,NULL};
    DunitUiTextV1 text={DUNIT_UI_ABI_VERSION,sizeof text,input,edited};
    DunitUiNumbersV1 numbers={DUNIT_UI_ABI_VERSION,sizeof numbers,configure,changed};
    DunitUiAppV1 app={DUNIT_UI_ABI_VERSION,sizeof app,&style,&text,&numbers};
    app.abi_version++;
    if(dunit_ui_run_app_v1(argc,(const char *const *)argv,(const char *const *)envp,560,520,&callbacks,&app)!=-DUNIT_EINVAL)return 3;
    app.abi_version--;m.invalid=1;
    if(dunit_ui_run_app_v1(argc,(const char *const *)argv,(const char *const *)envp,560,520,&callbacks,&app)!=-DUNIT_EINVAL)return 4;
    m.invalid=0;puts("[C-NUMBER] invalid configuration rejected");
    int result=dunit_ui_run_app_v1(argc,(const char *const *)argv,(const char *const *)envp,560,520,&callbacks,&app);
    if(result||m.resets!=1||m.h||m.v||m.spin||m.running||strcmp(m.note,"mixed"))return 5;
    puts("[C-NUMBER] ok");return 0;
}
