import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui

BarWidget {
  id: root
  moduleName: "omaaura-theme"

  readonly property string home: Quickshell.env("HOME")
  readonly property string scriptPath: {
    var localBin = decodeURIComponent(String(Qt.resolvedUrl("bin/omaaura-theme")).replace(/^file:\/\//, ""))
    return localBin
  }
  readonly property string paletteScriptPath: {
    var localBin = decodeURIComponent(String(Qt.resolvedUrl("bin/get-palette.py")).replace(/^file:\/\//, ""))
    return localBin
  }

  property string themeColor: "#89b4fa"
  property string rgbState: "on"
  property bool working: false
  property var themeColors: []
  property var backgroundColors: []

  readonly property bool showLabel: setting("showLabel", false) === true
  readonly property color foreground: bar ? bar.foreground : Color.foreground
  readonly property string fontFamily: bar ? bar.fontFamily : Style.font.family

  readonly property string tooltipText: "OmaAura: " + themeColor + " (" + (rgbState === "off" ? "Desligado" : "Ativo") + ")\nClique Esq: Paleta & Cores | Clique Dir: Ligar/Desligar"

  onTooltipTextChanged: if (mouseArea.containsMouse && bar && !popup.open) bar.showTooltip(root, tooltipText)

  function sync() {
    if (working) return
    working = true
    syncProcess.command = [root.scriptPath, "sync"]
    syncProcess.running = true
  }

  function toggle() {
    if (working) return
    working = true
    syncProcess.command = [root.scriptPath, "toggle"]
    syncProcess.running = true
  }

  function applyHex(hex) {
    if (working) return
    working = true
    syncProcess.command = [root.scriptPath, "set", hex]
    syncProcess.running = true
  }

  function reloadPalette() {
    if (!paletteProcess.running) {
      paletteProcess.running = true
    }
  }

  implicitWidth: content.implicitWidth + Style.space(10)
  implicitHeight: barSize

  // Observa alteração de cor de tema (keyboard.rgb)
  FileView {
    path: root.home + "/.local/state/omarchy/current/theme/keyboard.rgb"
    watchChanges: true
    printErrors: false
    onLoaded: {
      var raw = text().trim()
      if (raw.length > 0) {
        if (raw.indexOf("#") !== 0) raw = "#" + raw
        if (raw.length === 7) {
          root.themeColor = raw
        }
      }
      root.reloadPalette()
    }
    onFileChanged: {
      reload()
      root.reloadPalette()
    }
  }

  // Observa alteração do wallpaper atual
  FileView {
    path: root.home + "/.local/state/omarchy/current/background"
    watchChanges: true
    printErrors: false
    onLoaded: root.reloadPalette()
    onFileChanged: {
      reload()
      root.reloadPalette()
    }
  }

  // Observa alteração do estado ligado/desligado
  FileView {
    path: root.home + "/.local/state/omaaura-theme/state"
    watchChanges: true
    printErrors: false
    onLoaded: {
      var raw = text().trim()
      if (raw.indexOf("off") === 0) {
        root.rgbState = "off"
      } else {
        root.rgbState = "on"
        var parts = raw.split(":")
        if (parts.length > 1 && parts[1].length === 7) {
          root.themeColor = parts[1]
        }
      }
    }
    onFileChanged: reload()
  }

  Process {
    id: syncProcess
    command: []
    onExited: root.working = false
  }

  Process {
    id: paletteProcess
    command: ["python3", root.paletteScriptPath]
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        var raw = String(text || "").trim()
        if (!raw) return
        try {
          var data = JSON.parse(raw)
          if (data.theme) root.themeColors = data.theme
          if (data.background) root.backgroundColors = data.background
        } catch(e) {}
      }
    }
  }

  Component.onCompleted: {
    reloadPalette()
  }

  Row {
    id: content
    anchors.centerIn: parent
    spacing: Style.space(6)

    OpticalGlyph {
      anchors.verticalCenter: parent.verticalCenter
      width: Style.bar.iconSlot
      height: Style.bar.iconSlot
      text: root.rgbState === "off" ? "󰌶" : "󰌵"
      fontFamily: root.fontFamily
      fontSize: Style.font.icon
      color: root.rgbState === "off" ? Qt.darker(root.foreground, 1.8) : root.themeColor
      opacity: root.working ? 0.4 : 1.0

      Behavior on opacity { NumberAnimation { duration: 150 } }
      Behavior on color { ColorAnimation { duration: 250 } }
    }

    Text {
      anchors.verticalCenter: parent.verticalCenter
      visible: root.showLabel && !root.vertical
      text: root.themeColor
      color: root.foreground
      font.family: root.fontFamily
      font.pixelSize: Style.font.body
      renderType: Text.NativeRendering
    }
  }

  MouseArea {
    id: mouseArea
    anchors.fill: parent
    hoverEnabled: true
    acceptedButtons: Qt.LeftButton | Qt.RightButton
    cursorShape: Qt.PointingHandCursor
    onClicked: function(mouse) {
      if (mouse.button === Qt.RightButton) {
        root.toggle()
      } else {
        if (root.bar) root.bar.hideTooltip(root)
        root.reloadPalette()
        popup.open = !popup.open
      }
    }
    onEntered: if (root.bar && !popup.open) root.bar.showTooltip(root, root.tooltipText)
    onExited: if (root.bar) root.bar.hideTooltip(root)
  }

  PopupCard {
    id: popup
    anchorItem: root
    bar: root.bar
    triggerMode: "click"
    contentWidth: Style.space(310)
    contentHeight: popupColumn.implicitHeight + Style.space(24)

    Column {
      id: popupColumn
      anchors.fill: parent
      spacing: Style.space(12)

      // Cabeçalho com status e botão toggle
      Row {
        width: parent.width
        spacing: Style.space(8)

        OpticalGlyph {
          anchors.verticalCenter: parent.verticalCenter
          width: Style.bar.iconSlot
          height: Style.bar.iconSlot
          text: root.rgbState === "off" ? "󰌶" : "󰌵"
          fontFamily: root.fontFamily
          fontSize: Style.font.icon
          color: root.rgbState === "off" ? Qt.darker(root.foreground, 1.8) : root.themeColor
        }

        Column {
          anchors.verticalCenter: parent.verticalCenter
          width: parent.width - Style.bar.iconSlot - Style.space(16)

          Text {
            text: "OmaAura LED Hardware"
            color: Color.popups.text || root.foreground
            font.family: root.fontFamily
            font.pixelSize: Style.font.body
            font.bold: true
          }

          Text {
            text: "Cor ativa: " + root.themeColor + " (" + (root.rgbState === "off" ? "Desligado" : "Ligado") + ")"
            color: Qt.darker(Color.popups.text || root.foreground, 1.3)
            font.family: root.fontFamily
            font.pixelSize: Style.font.caption
          }
        }
      }

      Rectangle {
        width: parent.width
        height: 1
        color: Color.popups.border
      }

      // Seção 1: Cor predominante e destaques do Wallpaper
      Column {
        width: parent.width
        spacing: Style.space(6)

        Text {
          text: "WALLPAPER ATUAL (PREDOMINANTE)"
          color: Qt.darker(Color.popups.text || root.foreground, 1.3)
          font.family: root.fontFamily
          font.pixelSize: Style.font.caption
          font.bold: true
        }

        Row {
          spacing: Style.space(8)

          Repeater {
            model: root.backgroundColors

            Rectangle {
              required property var modelData
              width: Style.space(90)
              height: Style.space(38)
              radius: Style.cornerRadius
              color: modelData.displayHex || modelData.hex
              border.color: root.themeColor.toLowerCase() === modelData.hex.toLowerCase() ? (Color.popups.text || "#ffffff") : "transparent"
              border.width: 2

              MouseArea {
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: {
                  root.applyHex(modelData.hex)
                  popup.close()
                }

                Rectangle {
                  anchors.fill: parent
                  radius: Style.cornerRadius
                  color: "#ffffff"
                  opacity: parent.containsMouse ? 0.2 : 0.0
                }
              }

              Column {
                anchors.centerIn: parent
                spacing: Style.space(1)

                Text {
                  anchors.horizontalCenter: parent.horizontalCenter
                  text: modelData.name
                  color: "#ffffff"
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.caption * 0.85
                  font.bold: true
                  style: Text.Outline
                  styleColor: "#000000"
                }

                Text {
                  anchors.horizontalCenter: parent.horizontalCenter
                  text: modelData.hex.toUpperCase()
                  color: "#ffffff"
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.caption * 0.75
                  style: Text.Outline
                  styleColor: "#000000"
                }
              }
            }
          }
        }
      }

      // Seção 2: Cores do Tema Omarchy
      Column {
        width: parent.width
        spacing: Style.space(6)

        Text {
          text: "PALETA DO TEMA OMARCHY"
          color: Qt.darker(Color.popups.text || root.foreground, 1.3)
          font.family: root.fontFamily
          font.pixelSize: Style.font.caption
          font.bold: true
        }

        Grid {
          columns: 4
          spacing: Style.space(6)

          Repeater {
            model: root.themeColors

            Rectangle {
              required property var modelData
              width: Style.space(68)
              height: Style.space(32)
              radius: Style.cornerRadius
              color: modelData.hex
              border.color: root.themeColor.toLowerCase() === modelData.hex.toLowerCase() ? (Color.popups.text || "#ffffff") : "transparent"
              border.width: 2

              MouseArea {
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: {
                  root.applyHex(modelData.hex)
                  popup.close()
                }

                Rectangle {
                  anchors.fill: parent
                  radius: Style.cornerRadius
                  color: "#ffffff"
                  opacity: parent.containsMouse ? 0.2 : 0.0
                }
              }

              Text {
                anchors.centerIn: parent
                text: modelData.name
                color: "#ffffff"
                font.family: root.fontFamily
                font.pixelSize: Style.font.caption * 0.8
                font.bold: true
                style: Text.Outline
                styleColor: "#000000"
              }
            }
          }
        }
      }

      Rectangle {
        width: parent.width
        height: 1
        color: Color.popups.border
      }

      // Rodapé com ações de Sincronizar e Desligar
      Row {
        width: parent.width
        spacing: Style.space(8)

        Rectangle {
          width: (parent.width - Style.space(8)) / 2
          height: Style.space(28)
          radius: Style.cornerRadius
          color: Color.popups.border

          MouseArea {
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: {
              root.sync()
              popup.close()
            }
            Rectangle {
              anchors.fill: parent
              radius: Style.cornerRadius
              color: "#ffffff"
              opacity: parent.containsMouse ? 0.15 : 0.0
            }
          }

          Text {
            anchors.centerIn: parent
            text: "󰁪 Sincronizar"
            color: Color.popups.text || root.foreground
            font.family: root.fontFamily
            font.pixelSize: Style.font.caption
            font.bold: true
          }
        }

        Rectangle {
          width: (parent.width - Style.space(8)) / 2
          height: Style.space(28)
          radius: Style.cornerRadius
          color: root.rgbState === "off" ? Qt.darker(root.themeColor, 1.2) : Color.popups.border

          MouseArea {
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: {
              root.toggle()
              popup.close()
            }
            Rectangle {
              anchors.fill: parent
              radius: Style.cornerRadius
              color: "#ffffff"
              opacity: parent.containsMouse ? 0.15 : 0.0
            }
          }

          Text {
            anchors.centerIn: parent
            text: root.rgbState === "off" ? "󰌵 Ligar LEDs" : "󰌶 Desligar LEDs"
            color: Color.popups.text || root.foreground
            font.family: root.fontFamily
            font.pixelSize: Style.font.caption
            font.bold: true
          }
        }
      }
    }
  }
}
