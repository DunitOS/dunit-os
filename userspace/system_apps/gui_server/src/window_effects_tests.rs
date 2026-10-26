//! Host regressions import the production effect implementation. The painter
//! stub copies the test client's own surface, allowing lifetime assertions.
extern crate alloc;
use alloc::vec::Vec;
#[path = "animation.rs"] mod animation;
mod lua_policy {
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum Property { Opacity, X, Y, Width, Height, Scale, TranslationX, TranslationY, Blur, Shadow, CornerRadius, Color }
}
#[derive(Clone, Copy, Default)]
struct Theme { border_focused:u32,border_unfocused:u32,title_focused:u32,title_unfocused:u32 }
fn report_genie(_:u32,_:usize){}
struct Layout;
struct Font;
struct Applications;
#[derive(Clone, Copy, Default)]
struct Effects { blur:bool,blur_radius:i32,shadow:i32,corner_radius:i32 }
#[derive(Clone)]
struct Win { pid:u32,cx:i32,cy:i32,sw:i32,sh:i32,border:i32,title_h:i32,opacity:u8,
    alive:bool,minimized:bool,ws:usize,buf_ptr:*const u8,buf_size:usize,surface_w:i32,surface_h:i32 }
impl Win { fn outer(&self)->(i32,i32,i32,i32){(self.cx-self.border,self.cy-self.border-self.title_h,
    self.sw+2*self.border,self.sh+2*self.border+self.title_h)} }
fn blend_pixel(back:&mut[u32],bw:usize,bh:usize,x:i32,y:i32,pixel:u32){
    if x>=0 && y>=0 && (x as usize)<bw && (y as usize)<bh {back[y as usize*bw+x as usize]=pixel;}
}
mod window_painter {
    use super::*;
    pub fn paint(back:&mut[u32],_bw:usize,_bh:usize,w:&Win,_focused:bool,alpha:i32,
        _theme:&Theme,_layout:&Layout,_fx:&Effects,_font:Option<&Font>,_apps:&Applications,
        _mx:i32,_my:i32,_backing:&mut Vec<u32>,_logged:&mut bool,_a:&mut Vec<u32>,_b:&mut Vec<u32>){
        let count=(w.surface_w*w.surface_h)as usize;
        assert!(count*4<=w.buf_size);
        let pixels=unsafe{core::slice::from_raw_parts(w.buf_ptr as *const u32,count)};
        for (dst,src) in back.iter_mut().zip(pixels){*dst=((alpha as u32)<<24)|(src&0x00ff_ffff);}
    }
}
#[path = "window_effects.rs"] mod window_effects;
#[test]
fn properties_compose_independent_of_submission_order_and_commit_once(){
    use lua_policy::Property::*;
    let mut f=window_effects::Effects::new();
    assert!(!f.animating(10));
    let w=window();
    let unit=animation::UNIT as i64;
    for (property,from,to) in [(TranslationX,0,5),(Scale,unit,unit*2),(X,10,20),(Width,2,4),
        (Y,10,30),(TranslationY,0,-3),(Height,2,6)] {
        assert!(f.scalar(w.pid,property,10,100,from,to,animation::Easing::linear()));
    }
    assert!(f.animating(109));
    assert!(!f.animating(110));
    assert!(f.take_geometry_commits(&[w.clone()],109).is_empty());
    let commits=f.take_geometry_commits(&[w.clone()],110);
    assert_eq!(commits,alloc::vec![window_effects::GeometryCommit{pid:1,cx:25,cy:27,width:8,height:12}]);
    assert!(f.take_geometry_commits(&[w.clone()],111).is_empty());
    let mut changed=w; changed.cx=25;changed.cy=27;changed.sw=8;changed.sh=12;
    let (visual,_,_)=f.visual(&changed,111,&Theme::default(),&Effects::default());
    assert_eq!((visual.cx,visual.cy,visual.sw,visual.sh),(25,27,8,12));
}
#[test]
fn all_style_properties_keep_final_values_and_color_channels(){
    use lua_policy::Property::*;
    let mut f=window_effects::Effects::new();let w=window();
    for (property,from,to) in [(Opacity,65535,32768),(Blur,0,7),(Shadow,0,12),(CornerRadius,0,17),
        (Color,0xff000000u32 as i64,0xff80c0ffu32 as i64)] {
        assert!(f.scalar(1,property,1,100,from,to,animation::Easing::linear()));
    }
    let (_,theme,style)=f.visual(&w,u64::MAX,&Theme::default(),&Effects::default());
    assert_eq!(theme.title_focused,0xff80c0ff);
    assert!(style.blur);assert_eq!((style.blur_radius,style.shadow,style.corner_radius),(7,12,17));
    assert_eq!(f.alpha(1,u64::MAX),127);
    assert!(!f.scalar(1,Opacity,1,1,-1,1,animation::Easing::linear()));
}
#[test]
fn snapshots_survive_client_buffer_retirement_and_reverse(){
    let mut client=alloc::vec![0xff123456u32;4];let mut w=window();
    w.buf_ptr=client.as_ptr()as *const u8;w.buf_size=16;
    let theme=Theme::default();let fx=Effects::default();
    let style=window_effects::PaintStyle{theme:&theme,layout:&Layout,effects:&fx,font:None,apps:&Applications};
    let mut f=window_effects::Effects::new();
    assert!(!f.configure(1,animation::Effect::Fade,100,animation::Easing::linear(),0));
    f.start(&w,&[],0,0,(10,10,1,1),0,100,animation::Effect::Fade,animation::Easing::linear(),true,&style);
    assert!(f.covers(1,1));
    assert_eq!(f.snapshot_geometry(1),Some(((0,0,2,2),(10,10,1,1))));
    assert_eq!(f.snapshot_geometry(99),None);
    let mut other_workspace=alloc::vec![0u32;16*16];
    f.paint(&mut other_workspace,16,16,25,1);
    assert!(other_workspace.iter().all(|pixel|*pixel==0));
    client.fill(0xffabcdef);drop(client);
    let mut frame=alloc::vec![0u32;16*16];f.paint(&mut frame,16,16,25,0);
    let pixel=frame.iter().find(|p|**p!=0).copied().unwrap();
    assert_eq!(pixel&0x00ffffff,0x123456);
    let before=pixel>>24;
    w.minimized=true;f.restore(&w,25,100);
    frame.fill(0);f.paint(&mut frame,16,16,25,0);
    assert_eq!(frame.iter().find(|p|**p!=0).copied().unwrap()>>24,before);
    f.prune(&[w.clone()],1000);assert!(!f.covers(1,1000));
    frame.fill(0);f.paint(&mut frame,16,16,1000,0);assert!(frame.iter().all(|p|*p==0));
}
#[test]
fn completed_minimize_retained_only_for_live_minimized_window(){
    let pixels=alloc::vec![0xff123456u32;4];let mut w=window();w.buf_ptr=pixels.as_ptr()as *const u8;w.buf_size=16;
    let theme=Theme::default();let fx=Effects::default();
    let style=window_effects::PaintStyle{theme:&theme,layout:&Layout,effects:&fx,font:None,apps:&Applications};
    let mut f=window_effects::Effects::new();
    f.minimize(&w,&[],0,0,(10,10,1,1),0,100,&style);
    w.minimized=true;f.prune(&[w.clone()],1000);
    assert!(f.configure(1,animation::Effect::Scale,100,animation::Easing::linear(),1000));
    w.alive=false;f.prune(&[w],2000);
    assert!(!f.configure(1,animation::Effect::Fade,100,animation::Easing::linear(),2000));
    f.clear();
    assert!(!f.covers(1,2000));
}
#[test]
fn directional_targets_use_wide_arithmetic(){
    let (effect,curve)=window_effects::preset("genie",(0,0,10,10),(100,0,2,2));
    assert_eq!(effect,animation::Effect::Genie(animation::Edge::Right));
    assert_eq!(curve.sample(0),0);
    assert_eq!(window_effects::target_edge((i32::MIN,0,1,1),(i32::MAX,0,1,1)),animation::Edge::Right);
    assert_eq!(animation::Descriptor::new(animation::Effect::Fade,42,animation::Easing::linear()).unwrap().duration_ms(),42);
}
fn window()->Win {Win{pid:1,cx:0,cy:0,sw:2,sh:2,border:0,title_h:0,opacity:255,alive:true,minimized:false,ws:0,
    buf_ptr:core::ptr::null(),buf_size:0,surface_w:2,surface_h:2}}
