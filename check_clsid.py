import json

with open('C:/Users/chenzhihui/.ikun_organizer.json', 'r', encoding='utf-8') as f:
    config = json.load(f)

boxes = config.get('layout', {}).get('boxes', [])
for box in boxes:
    icon_names = box.get('icon_names', [])
    for i, icon in enumerate(icon_names):
        if '::' in icon:
            # Show exact bytes
            print(f'[{i}] icon={repr(icon)}')
            print(f'     bytes={icon.encode("utf-8").hex()}')
