path = r'D:\github-project\ikun_organizer\src\platform\windows\ffi_types.rs'
with open(path, 'r', encoding='utf-8') as f:
    content = f.read()

# Remove the S_OK definition I added
old = '''pub const S_OK: HRESULT = 0;
pub const DRAGDROP_S_DROP: HRESULT = 0x00040100;'''

new = 'pub const DRAGDROP_S_DROP: HRESULT = 0x00040100;'

if old in content:
    content = content.replace(old, new, 1)
    print('OK: Removed duplicate S_OK from ffi_types.rs')
else:
    print('SKIP: S_OK not found in ffi_types.rs')

with open(path, 'w', encoding='utf-8') as f:
    f.write(content)
