/* Native game-style input acceptance, compiled by TCC inside Dunit. */
#include <dunit.h>
struct Model { unsigned wanted, effective, revoked, denied, focused; int down,up,wheel_up,wheel_down; int left,right,top,bottom; long positive,negative,positive_y,negative_y; };
static int32_t build(void *u,DunitUiNodeV1 *n,uint32_t capacity){
    (void)u;if(capacity<1)return -DUNIT_EINVAL;
    const char *text="C SDK pointer modes";
    n[0]=(DunitUiNodeV1){DUNIT_UI_TEXT,DUNIT_UI_ROOT,(const uint8_t*)"title",5,(const uint8_t*)text,strlen(text),0};return 1;
}
static int32_t update(void *u,uint64_t m){(void)u;(void)m;return 0;}
static int32_t input(void *u,void *session,const DunitTransferEventV1 *e){
    struct Model *m=u;const DunitUiNativeInputV1 *i=e->input;
    if(!i)return 0;
    if(i->kind==DUNIT_GUI_IN_POINTER_MODE){
        m->effective=i->value;
        if(!i->x){
            if(i->y!=-DUNIT_EINTR||i->value!=DUNIT_POINTER_NORMAL)return -DUNIT_EINVAL;
            ++m->revoked;printf("[C-POINTER] revoked %u\n",m->revoked);return 0;
        }
        if(i->y){
            if(m->focused||i->y!=-DUNIT_EACCES||i->value!=DUNIT_POINTER_NORMAL)return -DUNIT_EINVAL;
            ++m->denied;puts("[C-POINTER] background denied");return 0;
        }
        if(i->value!=m->wanted)return -DUNIT_EINVAL;
        printf("[C-POINTER] mode %u\n",i->value);return 0;
    }
    if(i->kind==DUNIT_GUI_IN_FOCUS_IN){if(m->effective!=DUNIT_POINTER_NORMAL)return -DUNIT_EINVAL;m->focused=1;puts("[C-POINTER] focused");return 0;}
    if(i->kind==DUNIT_GUI_IN_FOCUS_OUT){
        m->focused=0;m->wanted=DUNIT_POINTER_LOCKED;
        return dunit_ui_pointer_mode_v1(session,m->wanted);
    }
    if(i->kind==DUNIT_GUI_IN_DOWN)m->down=1;
    if(i->kind==DUNIT_GUI_IN_UP)m->up=1;
    if(i->kind==DUNIT_GUI_IN_SCROLL){m->wheel_up|=i->x<0;m->wheel_down|=i->x>0;}
    if(i->kind==DUNIT_GUI_IN_RELATIVE){
        if(i->x>0)m->positive+=i->x;else m->negative+=i->x;
        if(i->y>0)m->positive_y+=i->y;else m->negative_y+=i->y;
        return 0;
    }
    if(i->kind==DUNIT_GUI_IN_MOVE&&m->wanted==DUNIT_POINTER_CONFINED){
        if(i->x<0||i->x>=400||i->y<0||i->y>=180)return -DUNIT_EINVAL;
        m->left|=i->x==0;m->right|=i->x==399;m->top|=i->y==0;m->bottom|=i->y==179;
    }
    if(i->kind!=DUNIT_GUI_IN_KEY)return 0;
    unsigned mode;
    switch(i->value){
    case 'c':mode=DUNIT_POINTER_CONFINED;break;
    case 'l':mode=DUNIT_POINTER_LOCKED;break;
    case 'n':mode=DUNIT_POINTER_NORMAL;break;
    case 'p':
        if(!m->left||!m->right||!m->top||!m->bottom)return -DUNIT_EINVAL;
        puts("[C-POINTER] all four confined edges checked");return 0;
    case 'v':
        printf("[C-POINTER] measured x=%ld,%ld y=%ld,%ld buttons=%d,%d wheel=%d,%d\n",m->positive,m->negative,m->positive_y,m->negative_y,m->down,m->up,m->wheel_up,m->wheel_down);
        if(m->positive<4000||m->negative>-4000||m->positive_y<4000||m->negative_y>-4000||!m->down||!m->up||!m->wheel_up||!m->wheel_down)return -DUNIT_EINVAL;
        printf("[C-POINTER] beyond screen positive=%ld negative=%ld y_positive=%ld y_negative=%ld\n",m->positive,m->negative,m->positive_y,m->negative_y);return 0;
    case 'k':
        if(m->revoked<3||m->denied<2||m->effective!=DUNIT_POINTER_LOCKED)return -DUNIT_EINVAL;
        puts("[C-POINTER] abrupt exit while locked");_Exit(0);
    default:return 0;
    }
    m->wanted=mode;int result=dunit_ui_pointer_mode_v1(session,mode);
    printf("[C-POINTER] request %u queued=%d\n",mode,result);return result;
}
int main(int argc,char **argv,char **envp){
    setvbuf(stdout,0,_IONBF,0);
    struct Model model={0};
    DunitUiCallbacksV1 callbacks={DUNIT_UI_ABI_VERSION,sizeof callbacks,&model,build,update};
    DunitTransfersV1 transfers={DUNIT_UI_ABI_VERSION,sizeof transfers,input};
    DunitUiExtendedV1 extended={DUNIT_UI_ABI_VERSION,sizeof extended,0,0,&transfers};
    int result=dunit_ui_run_extended_v1(argc,(const char *const*)argv,(const char *const*)envp,400,180,&callbacks,&extended);
    printf("[C-POINTER] FAIL run %d\n",result);return 1;
}
