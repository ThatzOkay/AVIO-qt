#[cfg(any(target_os = "windows", target_os = "linux"))]
use brightness::Brightness;
#[cfg(any(target_os = "windows", target_os = "linux"))]
use futures::TryStreamExt;

pub async fn get_current_brightness() -> Result<u32, String> {
    #[cfg(target_os = "macos")]
    {
        Ok(0)
    }
    #[cfg(target_os = "windows")]
    {
        let first_device = brightness::brightness_devices()
            .try_next()
            .await
            .map_err(|e| format!("Failed to get brightness devices: {e}"))?
            .ok_or_else(|| "No brightness devices found".to_string())?;

        let value = first_device
            .get()
            .await
            .map_err(|e| format!("Failed to get brightness: {e}"))?;
        Ok(value)
    }
    #[cfg(target_os = "linux")]
    {
        let first_device = brightness::brightness_devices()
            .try_next()
            .await
            .map_err(|e| format!("Failed to get brightness devices: {e}"))?
            .ok_or_else(|| "No brightness devices found".to_string())?;

        let value = first_device
            .get()
            .await
            .map_err(|e| format!("Failed to get brightness: {e}"))?;
        Ok(value)
    }
}

pub async fn set_brightness(value: u32) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        let mut first_device = brightness::brightness_devices()
            .try_next()
            .await
            .map_err(|e| format!("Failed to get brightness devices: {e}"))?
            .ok_or_else(|| "No brightness devices found".to_string())?;

        first_device
            .set(value)
            .await
            .map_err(|e| format!("Failed to set brightness: {e}"))?;
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        let mut first_device = brightness::brightness_devices()
            .try_next()
            .await
            .map_err(|e| format!("Failed to get brightness devices: {e}"))?
            .ok_or_else(|| "No brightness devices found".to_string())?;

        first_device
            .set(value)
            .await
            .map_err(|e| format!("Failed to set brightness: {e}"))?;
        Ok(())
    }
}
