import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui

Panel {
  id: root
  moduleName: "io.github.latex.omaaura-theme"
  ipcTarget: "io.github.latex.omaaura-theme"

  readonly property string home: Quickshell.env("HOME")
  readonly property string scriptPath: {
    var localBin = decodeURIComponent(String(Qt.resolvedUrl("bin/omaaura")).replace(/^file:\/\//, ""))
    return localBin
  }

  property string themeColor: "#89b4fa"
  property string rgbState: "on"
  property bool working: false
  property var themeColors: []
  property var backgroundColors: []

  readonly property bool showLabel: setting("showLabel", false) === true
  readonly property color foreground: bar ? bar.barForeground : Color.foreground
  readonly property string fontFamily: bar ? bar.fontFamily : Style.font.family

  // A cor do LED vem do wallpaper/estado e pode ser escura demais (ex.: #05121b),
  // sumindo contra a barra. Abaixo do limiar de luminância, cai para o foreground,
  // que por definição contrasta com o fundo da barra.
  readonly property color activeGlyphColor: {
    var c = Qt.color(root.themeColor)
    var lum = 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b
    return lum < 0.25 ? root.foreground : c
  }

  readonly property string tooltipText: "OmaAura: " + themeColor + " (" + (rgbState === "off" ? "Desligado" : "Ativo") + ")\nClique: Abrir Paleta | Clique Dir: Ligar/Desligar"

  function sync() {
    if (working) return
    working = true
    syncProcess.command = [root.scriptPath, "sync"]
    syncProcess.running = true
  }

  function toggleHardware() {
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

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  onOpenedChanged: if (opened) {
    reloadPalette()
    Qt.callLater(function() { catcher.forceActiveFocus() })
  }

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
    command: [root.scriptPath, "palette"]
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

  WidgetButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    // O conteúdo é um filho custom (Row/OpticalGlyph), não `text`. Sem isto o
    // WidgetButton calcula hasVisualContent=false e aplica opacity 0 (ícone some).
    hasVisualContent: true
    keepSpace: true
    labelVisible: false
    tooltipText: root.tooltipText
    fixedWidth: root.showLabel && !button.vertical ? -1 : Style.space(32)
    implicitWidth: root.showLabel && !button.vertical ? (content.implicitWidth + Style.space(12)) : Style.space(32)
    implicitHeight: button.barSize

    onPressed: function(mouseButton) {
      if (mouseButton === Qt.RightButton) {
        root.toggleHardware()
      } else {
        root.toggle()
      }
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
        color: root.rgbState === "off" ? Qt.darker(root.foreground, 1.8) : root.activeGlyphColor
        opacity: root.working ? 0.4 : 1.0

        Behavior on opacity { NumberAnimation { duration: 150 } }
        Behavior on color { ColorAnimation { duration: 250 } }
      }

      Text {
        anchors.verticalCenter: parent.verticalCenter
        visible: root.showLabel && !button.vertical
        text: root.themeColor
        color: root.foreground
        font.family: root.fontFamily
        font.pixelSize: Style.font.body
        renderType: Text.NativeRendering
      }
    }
  }

  KeyboardPanel {
    id: panel
    anchorItem: button
    bar: root.bar
    owner: root
    open: root.opened
    focusTarget: catcher
    contentWidth: panel.fittedContentWidth(Style.space(320))
    contentHeight: panel.fittedContentHeight(popupColumn.implicitHeight)

    PanelKeyCatcher {
      id: catcher
      anchors.fill: parent
      onCloseRequested: root.close()
      onTabRequested: function(direction) { root.switchPanel(direction) }

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
            color: root.rgbState === "off" ? Qt.darker(root.foreground, 1.8) : root.activeGlyphColor
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
          implicitHeight: 1
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
                implicitWidth: Style.space(90)
                implicitHeight: Style.space(38)
                radius: Style.cornerRadius
                // Mostra exatamente a cor que será calibrada e enviada ao LED.
                readonly property string ledColor: modelData.displayHex || modelData.hex
                color: ledColor
                border.color: root.themeColor.toLowerCase() === ledColor.toLowerCase() ? (Color.popups.text || "#ffffff") : "transparent"
                border.width: 2

                MouseArea {
                  anchors.fill: parent
                  hoverEnabled: true
                  cursorShape: Qt.PointingHandCursor
                  onClicked: {
                    root.applyHex(parent.ledColor)
                    root.close()
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
                    text: (modelData.displayHex || modelData.hex).toUpperCase()
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
                implicitWidth: Style.space(68)
                implicitHeight: Style.space(32)
                radius: Style.cornerRadius
                // Mostra exatamente a cor que será calibrada e enviada ao LED.
                readonly property string ledColor: modelData.displayHex || modelData.hex
                color: ledColor
                border.color: root.themeColor.toLowerCase() === ledColor.toLowerCase() ? (Color.popups.text || "#ffffff") : "transparent"
                border.width: 2

                MouseArea {
                  anchors.fill: parent
                  hoverEnabled: true
                  cursorShape: Qt.PointingHandCursor
                  onClicked: {
                    root.applyHex(parent.ledColor)
                    root.close()
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
          implicitHeight: 1
          color: Color.popups.border
        }

        // Rodapé com ações de Sincronizar e Desligar
        Row {
          width: parent.width
          spacing: Style.space(8)

          Rectangle {
            width: (parent.width - Style.space(8)) / 2
            height: Style.space(28)
            implicitWidth: (parent.width - Style.space(8)) / 2
            implicitHeight: Style.space(28)
            radius: Style.cornerRadius
            color: Color.popups.border

            MouseArea {
              anchors.fill: parent
              hoverEnabled: true
              cursorShape: Qt.PointingHandCursor
              onClicked: {
                root.sync()
                root.close()
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
            implicitWidth: (parent.width - Style.space(8)) / 2
            implicitHeight: Style.space(28)
            radius: Style.cornerRadius
            color: root.rgbState === "off" ? Qt.darker(root.themeColor, 1.2) : Color.popups.border

            MouseArea {
              anchors.fill: parent
              hoverEnabled: true
              cursorShape: Qt.PointingHandCursor
              onClicked: {
                root.toggleHardware()
                root.close()
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
}
