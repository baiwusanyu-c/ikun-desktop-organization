// 临时调试模块
#[cfg(test)]
mod debug_tests {
    use crate::platform::clsid_display_name;

    #[test]
    fn test_clsid_mapping() {
        let clsid = "::{5399E694-6CE5-4D6C-8FCE-1D8870FDCBA0}";
        let result = clsid_display_name(clsid);
        log::info!("CLSID {} -> {:?}", clsid, result);
        assert!(result.is_some(), "CLSID should map to a display name");
    }
}
