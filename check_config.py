import json
with open('C:/Users/chenzhihui/.ikun_organizer.json', 'r', encoding='utf-8') as f:
    config = json.load(f)

boxes = config.get('layout', {}).get('boxes', [])
for box in boxes:
    icon_names = box.get('icon_names', [])
    display_names = box.get('display_names', [])
    print(f'Box: {box.get("name", "?")}')
    print(f'  icon_names count: {len(icon_names)}')
    print(f'  display_names count: {len(display_names)}')
    for i, (icon, display) in enumerate(zip(icon_names, display_names)):
        if '::' in icon:
            print(f'  [{i}] icon={icon}')
            print(f'       display={repr(display)}')
