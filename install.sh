#!/bin/bash
set -e

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if ! command -v jq >/dev/null 2>&1; then
  echo "jq é necessário para ler o manifest. Instale com: omarchy pkg add jq" >&2
  exit 1
fi

PLUGIN_ID="$(jq -r '.id' "$PROJECT_DIR/manifest.json")"
TARGET_PLUGIN_DIR="$HOME/.config/omarchy/plugins/$PLUGIN_ID"

echo "=== Instalando $PLUGIN_ID ==="

# 1. Verificar OpenRGB
if ! command -v openrgb >/dev/null 2>&1; then
  echo "Instalando dependência openrgb..."
  omarchy pkg add openrgb
fi

# 2. Configurar pasta do plugin em ~/.config/omarchy/plugins/
echo "Copiando arquivos do plugin para $TARGET_PLUGIN_DIR..."
mkdir -p "$TARGET_PLUGIN_DIR"
cp -r "$PROJECT_DIR"/manifest.json "$PROJECT_DIR"/BarWidget.qml "$PROJECT_DIR"/README.md "$PROJECT_DIR"/LICENSE "$TARGET_PLUGIN_DIR"/
cp -r "$PROJECT_DIR"/omaaura "$TARGET_PLUGIN_DIR"/
mkdir -p "$TARGET_PLUGIN_DIR"/bin
cp -r "$PROJECT_DIR"/bin/* "$TARGET_PLUGIN_DIR"/bin/
chmod +x "$TARGET_PLUGIN_DIR"/bin/*
mkdir -p "$HOME/.local/bin"
ln -nsf "$TARGET_PLUGIN_DIR"/bin/omaaura "$HOME/.local/bin/omaaura"
ln -nsf "$TARGET_PLUGIN_DIR"/bin/omaaura-theme "$HOME/.local/bin/omaaura-theme"


# 3. Instalar Hooks do Omarchy
echo "Configurando hooks do Omarchy..."
mkdir -p "$HOME/.config/omarchy/hooks/theme-set.d" "$HOME/.config/omarchy/hooks/post-boot.d"
cp "$PROJECT_DIR"/hooks/theme-set "$HOME/.config/omarchy/hooks/theme-set.d/omaaura-theme.sh"
chmod +x "$HOME/.config/omarchy/hooks/theme-set.d/omaaura-theme.sh"
ln -nsf "$HOME/.config/omarchy/hooks/theme-set.d/omaaura-theme.sh" "$HOME/.config/omarchy/hooks/post-boot.d/omaaura-theme.sh"

# 4. Validar plugin
echo "Validando plugin..."
omarchy plugin validate "$TARGET_PLUGIN_DIR"

# 5. Ativar plugin e recarregar shell
echo "Ativando plugin $PLUGIN_ID..."
omarchy plugin enable "$PLUGIN_ID" --section right || true
omarchy-shell shell rescanPlugins >/dev/null 2>&1 || true

# 6. Sincronização inicial
echo "Sincronizando iluminação inicial..."
"$PROJECT_DIR"/bin/omaaura-theme sync || true

echo "=== Instalação concluída com sucesso! ==="
