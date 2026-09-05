import QtQuick
import QtQuick.Layouts
import org.pumpkin.gui

Rectangle {
    id: row

    // The `Players` QObject; actions go straight back to it.
    required property var controller
    required property string name
    required property int ping
    required property string dimension
    required property string gamemode
    required property real online

    signal reasonRequested(string action, string playerName)

    height: 34
    color: hover.hovered ? Theme.surfaceAlt : "transparent"

    HoverHandler {
        id: hover
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: 0
        anchors.rightMargin: 0
        spacing: 12

        Text {
            text: row.name
            color: Theme.fg
            font.pixelSize: 12
            elide: Text.ElideRight
            Layout.preferredWidth: 150
        }

        Text {
            text: row.ping + " ms"
            // Ping uses the same green/yellow/red scale as everything else, saturating at 300 ms.
            color: Theme.loadColor(row.ping / 300)
            font.pixelSize: 12
            font.family: "monospace"
            horizontalAlignment: Text.AlignRight
            Layout.preferredWidth: 60
        }

        Text {
            // `minecraft:overworld` is noise in a table; the namespace is almost always vanilla.
            text: row.dimension.replace("minecraft:", "")
            color: Theme.fgMuted
            font.pixelSize: 12
            elide: Text.ElideRight
            Layout.preferredWidth: 160
        }

        Text {
            text: row.gamemode
            color: Theme.fgMuted
            font.pixelSize: 12
            Layout.preferredWidth: 80
        }

        Text {
            text: Format.duration(row.online)
            color: Theme.fgMuted
            font.pixelSize: 12
            Layout.preferredWidth: 70
        }

        Item {
            Layout.fillWidth: true
        }

        // Dimmed until the row is hovered so the table reads as data rather than a control
        // panel, but not so faint that it is unclear the actions exist.
        RowLayout {
            spacing: 2
            opacity: hover.hovered ? 1 : 0.55
            Behavior on opacity {
                enabled: Theme.animations
                NumberAnimation {
                    duration: 120
                }
            }

            IconButton {
                source: Icons.op
                tint: hover.hovered ? Theme.accent : Theme.fg
                tooltip: qsTr("Grant operator")
                enabled: row.controller.hasCommands
                onClicked: row.controller.op(row.name)
            }

            IconButton {
                source: Icons.kick
                tint: hover.hovered ? Theme.warn : Theme.fg
                tooltip: qsTr("Kick…")
                enabled: row.controller.hasCommands
                onClicked: row.reasonRequested("kick", row.name)
            }

            IconButton {
                source: Icons.ban
                tint: hover.hovered ? Theme.danger : Theme.fg
                tooltip: qsTr("Ban…")
                enabled: row.controller.hasCommands
                onClicked: row.reasonRequested("ban", row.name)
            }
        }
    }

    Rectangle {
        anchors.bottom: parent.bottom
        width: parent.width
        height: 1
        color: Theme.border
        opacity: 0.5
    }
}
