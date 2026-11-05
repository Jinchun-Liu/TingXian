//! A single user-triggered full-display image, encoded in memory only.
use serde::Serialize;
use tokio_util::sync::CancellationToken;

#[cfg(windows)]
#[path = "interview_screenshot_windows.rs"]
mod windows;

#[derive(Serialize)]
pub struct Display { id:u32, width:u64, height:u64, primary:bool }

#[tauri::command]
pub fn interview_screenshot_displays()->Result<Vec<Display>,String> {
    #[cfg(target_os="macos")]
    {
        use core_graphics::display::CGDisplay;
        let main=CGDisplay::main().id;
        let ids=CGDisplay::active_displays().map_err(|_|"无法读取屏幕列表")?;
        Ok(ids.into_iter().map(|id|{let d=CGDisplay::new(id);Display{id,width:d.pixels_wide(),height:d.pixels_high(),primary:id==main}}).collect())
    }
    #[cfg(windows)]
    { windows::displays() }
    #[cfg(not(any(target_os="macos", windows)))]
    Err("当前平台尚未接入截屏".into())
}

fn bounded_size(width:usize,height:usize)->Result<(usize,usize),String> {
    if width==0 || height==0 {return Err("所选屏幕已断开或尺寸无效".into());}
    let ratio=(4096.0/(width.max(height) as f64)).min(1.0);
    Ok((((width as f64)*ratio).round().max(1.0) as usize,((height as f64)*ratio).round().max(1.0) as usize))
}

pub async fn capture(display_id:Option<u32>,cancel:CancellationToken)->Result<Vec<u8>,String> {
    if cancel.is_cancelled(){return Err("已取消".into());}
    #[cfg(target_os="macos")]
    {
        // ObjC objects stay on this worker; only PNG bytes leave it. A bounded
        // inner timeout also releases resources if the outer request is cancelled.
        let mut task=tokio::task::spawn_blocking(move ||cidre::objc::ar_pool(||{
            let runtime=tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|_|"无法启动截屏任务")?;
            runtime.block_on(async {tokio::time::timeout(std::time::Duration::from_secs(10),capture_mac(display_id)).await.map_err(|_|"截屏超过 10 秒，请检查屏幕录制权限后重试".to_string())?})
        }));
        tokio::select!{biased;
            _=cancel.cancelled()=>{let _=task.await;Err("已取消".into())},
            result=&mut task=>result.map_err(|_|"截屏任务意外中断".to_string())?,
        }
    }
    #[cfg(windows)]
    {
        let task = tokio::task::spawn_blocking(move || windows::capture(display_id));
        tokio::select! { biased;
            _ = cancel.cancelled() => Err("已取消".into()),
            result = tokio::time::timeout(std::time::Duration::from_secs(10), task) => {
                result.map_err(|_| "截屏超过 10 秒，请检查桌面会话后重试")?
                    .map_err(|_| "截屏任务意外中断")?
            }
        }
    }
    #[cfg(not(any(target_os="macos", windows)))]
    {let _=display_id;Err("当前平台尚未接入截屏".into())}
}

#[cfg(target_os="macos")]
async fn capture_mac(display_id:Option<u32>)->Result<Vec<u8>,String> {
    use cidre::{cf,cg,ns,sc};
    use core_graphics::display::CGDisplay;
    let id=display_id.unwrap_or_else(||CGDisplay::main().id);
    let displays=CGDisplay::active_displays().map_err(|_|"无法读取屏幕列表")?;
    if !displays.contains(&id){return Err("所选屏幕已断开，请刷新屏幕列表后重试".into());}
    let native=CGDisplay::new(id);
    let (width,height)=bounded_size(native.pixels_wide() as usize,native.pixels_high() as usize)?;
    let content=sc::ShareableContent::current().await.map_err(|_|"无法获取屏幕内容，请在 macOS 系统设置 → 隐私与安全性 → 屏幕录制中允许听弦，然后重启应用".to_string())?;
    let displays=content.displays();
    let display=displays.iter().find(|d|d.display_id().0==id).ok_or("没有找到所选屏幕，请检查屏幕录制权限或刷新列表")?;
    let filter=sc::ContentFilter::with_display_excluding_windows(&display,&ns::Array::new());
    let mut cfg=sc::StreamCfg::new();cfg.set_width(width);cfg.set_height(height);cfg.set_shows_cursor(false);cfg.set_scales_to_fit(true);
    let image=sc::ScreenshotManager::capture_image(&filter,&cfg).await.map_err(|_|"屏幕捕获失败，请检查屏幕录制权限后重试".to_string())?;
    let mut data=cf::DataMut::with_capacity(0);
    let png=cf::String::from_str("public.png");
    let mut dest=cg::ImageDst::with_data(&mut data,&png,1).ok_or("无法编码截图")?;
    dest.add_image(&image,None);
    if !dest.finalize(){return Err("截图编码失败".into());}
    if data.as_slice().len()>interview_core::MAX_SCREENSHOT_BYTES {return Err("截图超过 10 MiB，请降低所选屏幕分辨率后重试".into());}
    Ok(data.as_slice().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_frame_resize_keeps_aspect_without_upscaling_or_cropping(){
        assert_eq!(bounded_size(1920,1080).unwrap(),(1920,1080));
        assert_eq!(bounded_size(7680,4320).unwrap(),(4096,2304));
        assert_eq!(bounded_size(2880,5120).unwrap(),(2304,4096));
        assert!(bounded_size(0,1080).is_err());
    }
    #[tokio::test]
    async fn cancelled_request_never_starts_screen_capture(){
        let cancel=CancellationToken::new();cancel.cancel();assert_eq!(capture(None,cancel).await.unwrap_err(),"已取消");
    }
}
