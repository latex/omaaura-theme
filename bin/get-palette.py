#!/usr/bin/env python3
import json
import subprocess
import colorsys
import tomllib
from pathlib import Path

def normalize_color(hex_code, min_v=0.6, min_s=0.35):
    hex_code = hex_code.lstrip('#')
    r = int(hex_code[0:2], 16)/255.0
    g = int(hex_code[2:4], 16)/255.0
    b = int(hex_code[4:6], 16)/255.0
    h, s, v = colorsys.rgb_to_hsv(r, g, b)
    v_norm = max(min_v, v)
    s_norm = max(min_s, s) if s > 0.05 else s
    rf, gf, bf = colorsys.hsv_to_rgb(h, s_norm, v_norm)
    return f"#{int(round(rf*255)):02x}{int(round(gf*255)):02x}{int(round(bf*255)):02x}"

def get_palette():
    theme_colors = []
    theme_colors_path = Path.home() / '.local/state/omarchy/current/theme/colors.toml'
    if theme_colors_path.exists():
        try:
            data = tomllib.loads(theme_colors_path.read_text())
            keys = ['accent', 'red', 'orange', 'yellow', 'green', 'cyan', 'blue', 'magenta']
            for k in keys:
                if k in data and isinstance(data[k], str) and data[k].startswith('#'):
                    theme_colors.append({'name': k, 'hex': data[k].lower()})
        except Exception:
            pass

    bg_colors = []
    bg_path = Path.home() / '.local/state/omarchy/current/background'
    if bg_path.exists():
        try:
            real_bg = bg_path.resolve()
            cmd = ['magick', str(real_bg), '-scale', '64x64!', '-depth', '8', '+dither', '-colors', '16', '-format', '%c', 'histogram:info:']
            lines = subprocess.check_output(cmd, stderr=subprocess.DEVNULL).decode().splitlines()
            entries = []
            for line in lines:
                parts = line.strip().split()
                if len(parts) >= 3 and parts[2].startswith('#'):
                    hex_code = parts[2][1:7].lower()
                    count = int(parts[0].rstrip(':'))
                    r = int(hex_code[0:2], 16)/255.0
                    g = int(hex_code[2:4], 16)/255.0
                    b = int(hex_code[4:6], 16)/255.0
                    h, s, v = colorsys.rgb_to_hsv(r, g, b)
                    entries.append({'hex': '#' + hex_code, 'count': count, 'h': h, 's': s, 'v': v})
            
            if entries:
                # 1. Cor dominante mais frequente
                dom = max(entries, key=lambda x: x['count'])
                bg_colors.append({
                    'name': 'Fundo Dominante',
                    'hex': dom['hex'],
                    'displayHex': normalize_color(dom['hex'])
                })

                # 2. Cor de destaque / vibrante da cena
                vibrants = [e for e in entries if e['s'] > 0.15 and e['v'] > 0.15]
                if vibrants:
                    vib = max(vibrants, key=lambda x: x['count'] * (x['s']**0.7) * (x['v']**0.7))
                    if vib['hex'] != dom['hex']:
                        bg_colors.append({
                            'name': 'Destaque Wallpaper',
                            'hex': vib['hex'],
                            'displayHex': normalize_color(vib['hex'])
                        })

                # 3. Cor secundária frequente
                secondary = [e for e in entries if e['hex'] != dom['hex']]
                if secondary and len(bg_colors) < 3:
                    sec = max(secondary, key=lambda x: x['count'])
                    bg_colors.append({
                        'name': 'Secundária Wallpaper',
                        'hex': sec['hex'],
                        'displayHex': normalize_color(sec['hex'])
                    })
        except Exception:
            pass

    return {'theme': theme_colors, 'background': bg_colors}

if __name__ == '__main__':
    print(json.dumps(get_palette()))
