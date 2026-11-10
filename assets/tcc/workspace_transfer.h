/* Callback-scoped native SDK transfer demo; tokens are never kernel handles. */
static unsigned transfer_stage,transfer_done,drag_armed,drag_started,drag_done,drag_registered;
static uint64_t drag_request;
static uint64_t publication,reading,source_cap;
static int request(void *session,DunitTransferRequestV1 *r,DunitTransferResultV1 *out){return dunit_transfer_request_v1(session,r,out);}
static int publish(void *session,int files){
    DunitTransferRequestV1 r={0};DunitTransferResultV1 out;
    r.kind=DUNIT_TRANSFER_KIND_CLIPBOARD_OFFER;r.flags=DUNIT_TRANSFER_ACTION_COPY;r.preferred=DUNIT_TRANSFER_ACTION_COPY;
    if(files){r.files=&source_cap;r.files_len=1;}else{r.data=(const uint8_t *)"C clipboard UTF-8: cafe";r.data_len=strlen((const char *)r.data);}
    int e=request(session,&r,&out);publication=out.request;return e;
}
static int read_clipboard(void *session,int files){
    DunitTransferFormatV1 format={(const uint8_t *)(files?DUNIT_TRANSFER_FILES_MIME:DUNIT_TRANSFER_TEXT_MIME),0};format.mime_len=strlen((const char *)format.mime);
    DunitTransferRequestV1 r={0};DunitTransferResultV1 out;r.kind=DUNIT_TRANSFER_KIND_CLIPBOARD_READ;r.flags=DUNIT_TRANSFER_ACTION_COPY;r.preferred=DUNIT_TRANSFER_ACTION_COPY;r.formats=&format;r.formats_len=1;
    int e=request(session,&r,&out);reading=out.request;return e;
}
static int32_t transfer_callback(void *user,void *session,const DunitTransferEventV1 *event){
    DunitTransferRequestV1 r={0};DunitTransferResultV1 out;char buffer[64];int e;(void)user;
    if(transfer_done && rename_done && !drag_registered){
        DunitTransferFormatV1 format={(const uint8_t *)DUNIT_TRANSFER_TEXT_MIME,sizeof DUNIT_TRANSFER_TEXT_MIME-1};
        r.kind=DUNIT_TRANSFER_KIND_REGISTER_TARGET;r.flags=DUNIT_TRANSFER_ACTION_COPY;r.preferred=DUNIT_TRANSFER_ACTION_COPY;r.formats=&format;r.formats_len=1;
        if((e=request(session,&r,&out)))return e;drag_registered=1;
    }
    if(drag_registered && !drag_done && event->input){
        const DunitUiNativeInputV1 *input=event->input;
        if(input->kind==DUNIT_GUI_IN_DOWN && input->button==0)drag_armed=1;
        if(input->kind==DUNIT_GUI_IN_UP)drag_armed=0;
        if(input->kind==DUNIT_GUI_IN_MOVE && drag_armed && !drag_started){
            r.kind=DUNIT_TRANSFER_KIND_DRAG_OFFER;r.flags=DUNIT_TRANSFER_ACTION_COPY;r.preferred=DUNIT_TRANSFER_ACTION_COPY;r.data=(const uint8_t *)"C drag payload";r.data_len=14;
            if((e=request(session,&r,&out)))return e;drag_request=out.request;drag_started=1;
        }
    }
    if(drag_started && !drag_done && (event->kind==DUNIT_TRANSFER_KIND_DRAG_ENTER || event->kind==DUNIT_TRANSFER_KIND_DRAG_MOTION)){
        r.kind=DUNIT_TRANSFER_KIND_DRAG_ACCEPT;r.transfer=event->transfer;r.offset=0;r.preferred=DUNIT_TRANSFER_ACTION_COPY;
        if((e=request(session,&r,&out)))return e;puts("[C-TRANSFER] drag accepted");return 0;
    }
    if(drag_started && event->kind==DUNIT_TRANSFER_KIND_DATA && event->request==drag_request){
        size_t i;int found=0;for(i=0;i<event->representations_len;i++){const DunitTransferDataV1 *d=&event->representations[i];if(eq(d->mime,d->mime_len,DUNIT_TRANSFER_TEXT_MIME)&&eq(d->data,d->data_len,"C drag payload"))found=1;}
        if(!found)return -DUNIT_EIO;r.kind=DUNIT_TRANSFER_KIND_COMPLETE;r.transfer=event->transfer;r.flags=DUNIT_TRANSFER_STATUS_OK;
        if((e=request(session,&r,&out)))return e;drag_done=1;puts("[C-TRANSFER] real pointer drag drop round trip");return 0;
    }
    if(!transfer_stage){
        r.kind=DUNIT_C_FILE_OPEN;r.data=(const uint8_t *)"/tmp/c-cap-source";r.data_len=strlen((const char *)r.data);
        if((e=request(session,&r,&out)))return e;source_cap=out.capability;
        if(out.kind!=DUNIT_TRANSFER_FILE_KIND_FILE || (out.rights&DUNIT_RIGHT_WRITE))return -DUNIT_EACCES;
        r=(DunitTransferRequestV1){0};r.kind=DUNIT_C_FILE_READ;r.capability=source_cap;r.buffer=(uint8_t *)buffer;r.buffer_len=sizeof buffer;
        if((e=request(session,&r,&out)))return e;if(out.size!=18||memcmp(buffer,"capability payload",18))return -DUNIT_EIO;
        r=(DunitTransferRequestV1){0};r.kind=DUNIT_C_FILE_OPEN;r.data=(const uint8_t *)"/assets/tcc";r.data_len=strlen((const char *)r.data);
        if((e=request(session,&r,&out)))return e;uint64_t directory=out.capability;
        r=(DunitTransferRequestV1){0};r.kind=DUNIT_C_FILE_CHILD;r.capability=directory;r.flags=DUNIT_RIGHT_WRITE;r.data=(const uint8_t *)"workspace.c";r.data_len=11;
        if(request(session,&r,&out)!=-DUNIT_EPERM)return -DUNIT_EIO;
        r.flags=0;if((e=request(session,&r,&out)))return e;uint64_t child=out.capability;
        r=(DunitTransferRequestV1){0};r.kind=DUNIT_C_FILE_STAT;r.capability=child;if((e=request(session,&r,&out)))return e;if(!out.size)return -DUNIT_EIO;
        r.kind=DUNIT_C_FILE_CLOSE;if((e=request(session,&r,&out)))return e;r.kind=DUNIT_C_FILE_STAT;if(request(session,&r,&out)!=-DUNIT_EBADF)return -DUNIT_EIO;
        r.kind=DUNIT_C_FILE_CLOSE;r.capability=directory;if((e=request(session,&r,&out)))return e;
        puts("[C-TRANSFER] real file directory capability rights and stale token");
        transfer_stage=1;return 0;
    }
    if(transfer_stage==1 && !publication && event->input && (event->input->kind==DUNIT_GUI_IN_FOCUS_IN || event->input->kind==DUNIT_GUI_IN_KEY || event->input->kind==DUNIT_GUI_IN_UP))return publish(session,0);
    if(event->kind==DUNIT_TRANSFER_KIND_RESULT && event->request==publication){
        if(event->arg0!=DUNIT_TRANSFER_STATUS_OK)return -DUNIT_EIO;
        return read_clipboard(session,transfer_stage==2);
    }
    if(event->kind==DUNIT_TRANSFER_KIND_DATA && event->request==reading){
        if(transfer_stage==1){
            size_t i;int found=0;for(i=0;i<event->representations_len;i++){const DunitTransferDataV1 *d=&event->representations[i];if(eq(d->mime,d->mime_len,DUNIT_TRANSFER_TEXT_MIME)&&eq(d->data,d->data_len,"C clipboard UTF-8: cafe"))found=1;}
            if(!found)return -DUNIT_EIO;puts("[C-TRANSFER] text clipboard round trip");
        }else{
            if(event->files_len!=1||event->files[0].kind!=DUNIT_TRANSFER_FILE_KIND_FILE||(event->files[0].rights&DUNIT_RIGHT_WRITE))return -DUNIT_EIO;
            r.kind=DUNIT_C_FILE_READ;r.capability=event->files[0].capability;r.buffer=(uint8_t *)buffer;r.buffer_len=sizeof buffer;
            if((e=request(session,&r,&out)))return e;if(out.size!=18||memcmp(buffer,"capability payload",18))return -DUNIT_EIO;
            r=(DunitTransferRequestV1){0};r.kind=DUNIT_C_FILE_CLOSE;r.capability=event->files[0].capability;if((e=request(session,&r,&out)))return e;
            puts("[C-TRANSFER] file clipboard real attenuated capability");transfer_done=1;
        }
        r=(DunitTransferRequestV1){0};r.kind=DUNIT_TRANSFER_KIND_COMPLETE;r.transfer=event->transfer;r.flags=DUNIT_TRANSFER_STATUS_OK;if((e=request(session,&r,&out)))return e;
        if(transfer_stage==1){transfer_stage=2;return publish(session,1);}
        r=(DunitTransferRequestV1){0};r.kind=DUNIT_C_FILE_CLOSE;r.capability=source_cap;if((e=request(session,&r,&out)))return e;
        r=(DunitTransferRequestV1){0};r.kind=DUNIT_TRANSFER_KIND_CLIPBOARD_CLEAR;return request(session,&r,&out);
    }
    return 0;
}
