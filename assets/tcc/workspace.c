/* Build on-device: tcc workspace.c -o /tmp/c-gui -ldunit_ui */
#include <dunit.h>
#include <stdio.h>
#include <string.h>
#define SPAN(s) (const uint8_t *)(s), sizeof(s)-1
#define ITEM(k,l,p,f) {SPAN(k),SPAN(l),SPAN(p),f,0}
static DunitUiCollectionItemV1 items[]={ITEM("a","Alpha","",0),ITEM("b","Beta","",0),ITEM("c","Disabled","",DUNIT_UI_ITEM_DISABLED)};
static DunitUiCollectionItemV1 trees[]={ITEM("folder","Folder","",0),ITEM("child","Child","folder",0)};
static DunitUiCollectionItemV1 rows[]={ITEM("small","","",0),ITEM("large","","",0)};
static DunitUiTableColumnV1 columns[]={ {SPAN("name"),SPAN("Name"),260,DUNIT_UI_COLUMN_SORTABLE,0},{SPAN("size"),SPAN("Size"),160,DUNIT_UI_COLUMN_SORTABLE,0} };
static DunitUiTableCellV1 cells[]={ {SPAN("Small"),0,DUNIT_UI_CELL_TEXT,0},{NULL,0,2,DUNIT_UI_CELL_INTEGER,0},{SPAN("Large"),0,DUNIT_UI_CELL_TEXT,0},{NULL,0,100,DUNIT_UI_CELL_UNSIGNED,0} };
static unsigned selected,dialog,sort_desc,sorted,sort_size,column_size,changes,split_changes;
static int64_t split_value=50;
static char status[100],rename_value[256];
static unsigned renaming,rename_done;
static DunitUiRenameV1 rename_state;
static int eq(const uint8_t *p,size_t n,const char *s){return strlen(s)==n && !memcmp(p,s,n);}

#include "workspace_transfer.h"
static int32_t configure(void *u,uint32_t index,DunitUiCollectionV1 *c){
    (void)u;
    if(index==3){c->items=items;c->items_len=3;c->flags=DUNIT_UI_COLLECTION_MULTIPLE;if(renaming){rename_state=(DunitUiRenameV1){0,0,SPAN("a"),NULL,0,(const uint8_t *)rename_value,strlen(rename_value)};c->rename=&rename_state;}}
    else if(index==4){c->items=trees;c->items_len=2;}
    else if(index==5){c->items=rows;c->items_len=2;c->columns=columns;c->columns_len=2;c->cells=cells;c->cells_len=4;
        if(sorted){c->sort=(const uint8_t *)(sort_size?"size":"name");c->sort_len=4;}
        if(column_size){c->column=(const uint8_t *)"size";c->column_len=4;}
        if(sort_desc)c->flags|=DUNIT_UI_COLLECTION_SORT_DESCENDING;
    }else return -DUNIT_EINVAL;
    return 0;
}
static int32_t changed(void *u,uint64_t message,const DunitUiCollectionV1 *s){
    DunitUiCollectionItemV1 *target;size_t count,i,j;(void)u;
    if(message==1){target=items;count=3;}else if(message==2){target=trees;count=2;}else if(message==3){target=rows;count=2;}else return -DUNIT_EINVAL;
    for(i=0;i<count;i++)target[i].flags &= DUNIT_UI_ITEM_DISABLED;
    for(i=0;i<s->items_len;i++)for(j=0;j<count;j++)if(s->items[i].key_len==target[j].key_len&&!memcmp(s->items[i].key,target[j].key,target[j].key_len))target[j].flags|=s->items[i].flags;
    if(message==3){column_size=eq(s->column,s->column_len,"size");sorted=s->sort_len!=0;sort_size=eq(s->sort,s->sort_len,"size");sort_desc=(s->flags&DUNIT_UI_COLLECTION_SORT_DESCENDING)!=0;}
    changes++;
    if(message==1 && (items[0].flags&DUNIT_UI_ITEM_SELECTED)&&(items[1].flags&DUNIT_UI_ITEM_SELECTED))puts("[C-WORKSPACE] multiple selection");
    if(message==2 && (trees[0].flags&DUNIT_UI_ITEM_EXPANDED)&&(trees[1].flags&DUNIT_UI_ITEM_CURRENT))puts("[C-WORKSPACE] expanded child selected");
    if(message==3 && sorted && column_size && sort_desc)puts("[C-WORKSPACE] numeric column descending");
    return 0;
}
static int32_t renamed(void *u,uint64_t id,const DunitUiRenameV1 *event){(void)u;if(id!=1)return -DUNIT_EINVAL;
    if(event->kind==DUNIT_UI_RENAME_BEGIN){if(!eq(event->row,event->row_len,"a"))return -DUNIT_EINVAL;if(items[0].label_len>=sizeof rename_value)return -DUNIT_ENOBUFS;memcpy(rename_value,items[0].label,items[0].label_len);rename_value[items[0].label_len]=0;renaming=1;}
    else if(event->kind==DUNIT_UI_RENAME_CHANGE){if(event->value_len>=sizeof rename_value)return -DUNIT_ENOBUFS;memcpy(rename_value,event->value,event->value_len);rename_value[event->value_len]=0;}
    else if(event->kind==DUNIT_UI_RENAME_COMMIT){if(strcmp(rename_value,"renamed"))return -DUNIT_EINVAL;items[0].label=(const uint8_t *)"renamed";items[0].label_len=7;renaming=0;rename_done=1;puts("[C-WORKSPACE] controlled rename committed");}
    else if(event->kind==DUNIT_UI_RENAME_CANCEL)renaming=0;else return -DUNIT_EINVAL;return 0;
}
static int32_t number_config(void *u,uint32_t i,DunitUiNumberV1 *n){(void)u;if(i!=15)return -DUNIT_EINVAL;n->min=0;n->max=100;n->step=10;n->page_step=20;n->value=split_value;return 0;}
static int32_t number_changed(void *u,uint64_t id,int64_t value){(void)u;if(id!=9)return -DUNIT_EINVAL;split_value=value;split_changes++;puts("[C-WORKSPACE] splitter changed");return 0;}
static int32_t update(void *u,uint64_t id){(void)u;
    if(id==10||id==11){selected=id==11;puts("[C-WORKSPACE] tab changed");}
    else if(id==12){dialog=1;puts("[C-WORKSPACE] modal opened");}
    else if(id==13){dialog=0;puts("[C-WORKSPACE] modal closed");}
    else if(id==14){if(!drag_done||!rename_done||!transfer_done||changes!=10||split_changes!=1||split_value!=60||!selected||dialog)return -DUNIT_EINVAL;puts("[C-WORKSPACE] ok");return 1;}
    else return -DUNIT_EINVAL;return 0;
}
static int32_t build(void *u,DunitUiNodeV1 *n,uint32_t cap){unsigned count=0;(void)u;if(cap<25)return -DUNIT_EINVAL;
#define NODE(k,p,key,label,id) n[count++]=(DunitUiNodeV1){k,p,SPAN(key),SPAN(label),id}
    NODE(DUNIT_UI_MODAL,DUNIT_UI_ROOT,"host","",0);
    NODE(DUNIT_UI_COLUMN,0,"body","",0);
    NODE(DUNIT_UI_TEXT,1,"title","C SDK workspace + collections",0);
    NODE(DUNIT_UI_LIST,1,"list","Project list",1);
    NODE(DUNIT_UI_TREE,1,"tree","Project tree",2);
    NODE(DUNIT_UI_TABLE,1,"table","Files",3);
    NODE(DUNIT_UI_TABS,1,"tabs","Pages",0);
    NODE(selected?DUNIT_UI_TAB:DUNIT_UI_TAB_SELECTED,6,"overview","Overview",10);
    NODE(DUNIT_UI_TEXT,7,"first","The SDK owns tabs and focus",0);
    NODE(selected?DUNIT_UI_TAB_SELECTED:DUNIT_UI_TAB,6,"details","Details",11);
    NODE(DUNIT_UI_SCROLL_BOTH,9,"details-scroll","Details",0);
    NODE(DUNIT_UI_TOOLBAR,1,"bar","Actions",0);
    NODE(DUNIT_UI_BUTTON,11,"open","About",12);
    NODE(DUNIT_UI_TOOLBAR_SEPARATOR,11,"separator","",0);
    NODE(DUNIT_UI_BUTTON,11,"quit","Quit",14);
    NODE(DUNIT_UI_SPLITTER,1,"split","Panes",9);
    NODE(DUNIT_UI_TEXT,15,"left","Left pane",0);
    NODE(DUNIT_UI_TEXT,15,"right","Right pane",0);
    snprintf(status,sizeof status,"Changes %u / split %lld",changes,(long long)split_value);
    n[count++]=(DunitUiNodeV1){DUNIT_UI_STATUS_BAR,1,SPAN("status"),(const uint8_t *)status,strlen(status),0};
    if(dialog){
        NODE(DUNIT_UI_MESSAGE_BOX,0,"about","One SDK, two languages",0);
        NODE(DUNIT_UI_MESSAGE_BODY,19,"message","Same widgets. Same theme.\nCompiled by TCC inside Dunit.",0);
        NODE(DUNIT_UI_DIALOG_ACTION,19,"dismiss","Close",13);
        NODE(DUNIT_UI_DIALOG_DEFAULT,19,"default","dismiss",0);
        NODE(DUNIT_UI_DIALOG_CANCEL,19,"cancel","",13);
    }
    return count;
}
int main(int argc,char **argv,char **envp){
    setvbuf(stdout,NULL,_IONBF,0);
    DunitUiCallbacksV1 cb={DUNIT_UI_ABI_VERSION,sizeof cb,NULL,build,update};
    DunitUiCollectionsV1 collections={DUNIT_UI_ABI_VERSION,sizeof collections,configure,changed,renamed};
    DunitUiNumbersV1 numbers={DUNIT_UI_ABI_VERSION,sizeof numbers,number_config,number_changed};
    char stylesheet[4096];FILE *f=fopen("/assets/tcc/workspace.dss","r");size_t len;if(!f)return 1;len=fread(stylesheet,1,sizeof stylesheet,f);fclose(f);
    DunitUiStyleV1 style={DUNIT_UI_ABI_VERSION,sizeof style,(const uint8_t *)stylesheet,len,NULL};
    DunitUiAppV1 app={DUNIT_UI_ABI_VERSION,sizeof app,&style,NULL,&numbers};
    FILE *fixture=fopen("/tmp/c-cap-source","w");if(!fixture)return 1;fwrite("capability payload",1,18,fixture);fclose(fixture);
    DunitTransfersV1 transfers={DUNIT_UI_ABI_VERSION,sizeof transfers,transfer_callback};
    DunitUiExtendedV1 extended={DUNIT_UI_ABI_VERSION,sizeof extended,&app,&collections,&transfers};
    int result=dunit_ui_run_extended_v1(argc,(const char *const *)argv,(const char *const *)envp,640,620,&cb,&extended);
    if(result){printf("[C-WORKSPACE] failure %d\n",result);return 1;}return 0;
}
