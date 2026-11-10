/* Declarative SDK popups; no C overlay state, geometry, timer or painter. */
#include <dunit.h>
struct Model { unsigned actions, selected, choices, declined, resets; int invalid; char status[96]; };
static DunitUiNodeV1 node(uint32_t kind,uint32_t parent,const char *key,const char *text,uint64_t id)
{ DunitUiNodeV1 n={kind,parent,(const uint8_t *)key,strlen(key),(const uint8_t *)text,strlen(text),id};return n; }
static int32_t build(void *user,DunitUiNodeV1 *n,uint32_t capacity)
{
    struct Model *m=user;if(capacity<28)return -DUNIT_EINVAL;
    snprintf(m->status,sizeof m->status,"Actions %u / color %s",m->actions,m->selected?"blue":"red");
    n[0]=node(DUNIT_UI_COLUMN,DUNIT_UI_ROOT,"root","",0);n[1]=node(DUNIT_UI_TEXT,0,"title","C SDK menus and popups",0);
    n[2]=node(DUNIT_UI_MENU_BAR,0,"bar","",0);n[3]=node(DUNIT_UI_MENU,2,"file","File",0);n[4]=node(DUNIT_UI_MNEMONIC,3,"file-key","f",0);
    n[5]=node(DUNIT_UI_MENU_ITEM,3,"new","New",10);n[6]=node(DUNIT_UI_MENU_ITEM|DUNIT_UI_DISABLED,3,"unavailable","Unavailable",999);
    n[7]=node(DUNIT_UI_MENU_SEPARATOR,3,"separator","",0);n[8]=node(DUNIT_UI_SUBMENU,3,"recent","Recent",0);n[9]=node(DUNIT_UI_MNEMONIC,8,"recent-key","r",0);n[10]=node(DUNIT_UI_MENU_ITEM,8,"project","Project",UINT64_MAX);
    n[11]=node(DUNIT_UI_MENU,2,"tools","Tools",0);n[12]=node(DUNIT_UI_MNEMONIC,11,"tools-key","t",0);n[13]=node(DUNIT_UI_MENU_ITEM,11,"tools-reset","Reset",20);
    n[14]=node(DUNIT_UI_SELECT,0,"color","Choose color",0);
    n[15]=node(!m->selected||m->invalid?DUNIT_UI_OPTION_SELECTED:DUNIT_UI_OPTION,14,"red","Red",30);
    n[16]=node(DUNIT_UI_OPTION|DUNIT_UI_DISABLED,14,"green","Green unavailable",999);
    n[17]=node(m->selected||m->invalid?DUNIT_UI_OPTION_SELECTED:DUNIT_UI_OPTION,14,"blue","Blue",31);
    n[18]=node(DUNIT_UI_BUTTON,0,"surface","Context actions (Shift+F10)",51);
    n[19]=node(DUNIT_UI_CONTEXT_MENU,18,"context-binding","",0);n[20]=node(DUNIT_UI_MENU_ITEM,19,"inspect","Inspect",40);n[21]=node(DUNIT_UI_MENU_ITEM|DUNIT_UI_DISABLED,19,"context-disabled","Unavailable",999);
    n[22]=node(DUNIT_UI_BUTTON,0,"help","Hover for SDK tooltip",50);n[23]=node(DUNIT_UI_TOOLTIP,22,"help-tooltip","Tooltips use the shared SDK timer",0);
    n[24]=node(DUNIT_UI_ROW,0,"actions","",0);n[25]=node(DUNIT_UI_BUTTON,24,"reset","Reset",20);n[26]=node(DUNIT_UI_BUTTON,24,"quit","Quit",8);n[27]=node(DUNIT_UI_TEXT,0,"status",m->status,0);return 28;
}
static int32_t update(void *user,uint64_t id)
{
    struct Model *m=user;
    if(id==8)return 1;
    if(id==10){m->actions++;puts("[C-POPUP] menu action");}
    else if(id==UINT64_MAX){m->actions++;puts("[C-POPUP] submenu action");}
    else if(id==31){m->selected=1;m->choices++;puts("[C-POPUP] selected blue");}
    else if(id==30&&m->selected){m->declined++;puts("[C-POPUP] selection declined");}
    else if(id==40){m->actions++;puts("[C-POPUP] context action");}
    else if(id==20){if(m->actions!=3||m->choices!=1||m->declined!=1||!m->selected)return -DUNIT_EINVAL;m->actions=0;m->selected=0;m->resets++;puts("[C-POPUP] reset");}
    else return -DUNIT_EINVAL;
    return 0;
}
int main(int argc,char **argv,char **envp)
{
    setvbuf(stdout,NULL,_IONBF,0);FILE *f=fopen("/assets/tcc/popups.dss","rb");if(!f)return 1;
    char sheet[8192];size_t length=fread(sheet,1,sizeof sheet,f);if(ferror(f)||!feof(f)){fclose(f);return 2;}fclose(f);
    struct Model model={0};DunitUiCallbacksV1 callbacks={DUNIT_UI_ABI_VERSION,sizeof callbacks,&model,build,update};DunitUiStyleV1 style={DUNIT_UI_ABI_VERSION,sizeof style,(const uint8_t *)sheet,length,NULL};
    model.invalid=1;if(dunit_ui_run_styled_v1(argc,(const char *const *)argv,(const char *const *)envp,560,420,&callbacks,&style)!=-DUNIT_EINVAL)return 3;
    model.invalid=0;puts("[C-POPUP] invalid selection rejected");
    int result=dunit_ui_run_styled_v1(argc,(const char *const *)argv,(const char *const *)envp,560,420,&callbacks,&style);
    if(result||model.resets!=1||model.actions||model.selected||model.choices!=1||model.declined!=1)return 4;
    puts("[C-POPUP] ok");return 0;
}
