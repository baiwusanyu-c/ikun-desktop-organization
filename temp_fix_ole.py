import os

# Fix 1: Add S_OK constant to ffi_types.rs
path1 = r'D:\github-project\ikun_organizer\src\platform\windows\ffi_types.rs'
with open(path1, 'r', encoding='utf-8') as f:
    content1 = f.read()

old1 = 'pub const DRAGDROP_S_DROP: HRESULT = 0x00040100;'
new1 = '''pub const S_OK: HRESULT = 0;
pub const DRAGDROP_S_DROP: HRESULT = 0x00040100;'''

if old1 in content1:
    content1 = content1.replace(old1, new1, 1)
    with open(path1, 'w', encoding='utf-8') as f:
        f.write(content1)
    print('OK: Added S_OK constant')
else:
    print('SKIP: S_OK already exists or pattern not found')

# Fix 2: Modify drop_source_give_feedback to update drag overlay
path2 = r'D:\github-project\ikun_organizer\src\platform\windows\ole_dnd.rs'
with open(path2, 'r', encoding='utf-8') as f:
    content2 = f.read()

old2 = '''unsafe extern "system" fn drop_source_give_feedback(_this: *mut c_void, _effect: u32) -> HRESULT {
    DRAGDROP_S_USEDEFAULTCURSORS
}'''

new2 = '''unsafe extern "system" fn drop_source_give_feedback(_this: *mut c_void, _effect: u32) -> HRESULT {
    // \u66f4\u65b0\u62d6\u62fd\u6b8b\u5f71\u4f4d\u7f6e\uff0c\u4fdd\u6301\u6b8b\u5f71\u8ddf\u968f\u5149\u6807
    super::drag_image::update_drag_overlay();
    // \u8fd4\u56de S_OK \u8868\u793a\u6211\u4eec\u81ea\u5df1\u5904\u7406\u4e86\u53cd\u9988\uff08\u4e0d\u4f7f\u7528\u7cfb\u7edf\u9ed8\u8ba4\u5149\u6807\uff09
    S_OK
}'''

if old2 in content2:
    content2 = content2.replace(old2, new2, 1)
    with open(path2, 'w', encoding='utf-8') as f:
        f.write(content2)
    print('OK: Modified drop_source_give_feedback')
else:
    print('ERROR: Could not find drop_source_give_feedback')

print('Fixes 1 & 2 applied')
