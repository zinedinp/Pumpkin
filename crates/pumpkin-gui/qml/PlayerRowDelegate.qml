import QtQuick
import QtQuick.Layouts
import org.pumpkin.gui

Rectangle {
    id: row

    // The `Players` QObject; actions go straight back to it.
    required property var controller
    required property string view
    required property string name
    required property string uuid
    required property int ping
    required property string dimension
    required property string gamemode
    required property real online
    required property bool isOnline
    required property bool operator
    required property bool banned
    required property bool whitelisted
    required property int index

    signal reasonRequested(string action, string playerName)

    height: 48
    color: hover.hovered ? Theme.rowHover : (index % 2 === 0 ? Theme.rowEven : Theme.rowOdd)

    HoverHandler {
        id: hover
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: 0
        anchors.rightMargin: 0
        spacing: 12

        CopyableText {
            compact: true
            mono: false
            value: row.name
            Layout.preferredWidth: 186
            Layout.minimumWidth: 186
            Layout.maximumWidth: 186
            Layout.fillWidth: false
            Layout.alignment: Qt.AlignVCenter
        }

        CopyableText {
            compact: true
            value: row.uuid
            Layout.preferredWidth: 276
            Layout.minimumWidth: 276
            Layout.maximumWidth: 276
            Layout.fillWidth: false
            Layout.alignment: Qt.AlignVCenter
        }

        Text {
            text: row.isOnline ? (row.ping + " ms") : "–"
            color: row.isOnline ? Theme.loadColor(row.ping / 300) : Theme.fgMuted
            font.pixelSize: 12
            font.family: "monospace"
            horizontalAlignment: Text.AlignRight
            Layout.preferredWidth: 60
        }

        Text {
            text: row.dimension === "" ? "–" : row.dimension.replace("minecraft:", "")
            color: Theme.fgMuted
            font.pixelSize: 12
            elide: Text.ElideRight
            Layout.preferredWidth: 120
        }

        Text {
            text: row.gamemode === "" ? "–" : row.gamemode
            color: Theme.fgMuted
            font.pixelSize: 12
            Layout.preferredWidth: 80
        }

        Text {
            text: row.isOnline ? Format.duration(row.online) : "–"
            color: Theme.fgMuted
            font.pixelSize: 12
            Layout.preferredWidth: 70
        }

        Item {
            Layout.fillWidth: true
        }

        RowLayout {
            spacing: 6

            IconButton {
                visible: row.view === "online" || row.view === "offline"
                source: Icons.op
                tint: Theme.accent
                tooltip: qsTr("Grant operator")
                enabled: row.controller.hasCommands
                onClicked: row.controller.op(row.name)
            }

            IconButton {
                visible: row.view === "online"
                source: Icons.kick
                tint: Theme.warn
                tooltip: qsTr("Kick…")
                enabled: row.controller.hasCommands
                onClicked: row.reasonRequested("kick", row.name)
            }

            IconButton {
                visible: row.view === "online" || row.view === "offline"
                source: Icons.ban
                tint: Theme.danger
                tooltip: qsTr("Ban…")
                enabled: row.controller.hasCommands
                onClicked: row.reasonRequested("ban", row.name)
            }

            IconButton {
                visible: row.view === "operator"
                source: Icons.op
                tint: Theme.danger
                tooltip: qsTr("Revoke operator")
                enabled: row.controller.hasCommands
                onClicked: row.controller.deop(row.name)
            }

            ThemedButton {
                visible: row.view === "banned"
                text: qsTr("Pardon")
                enabled: row.controller.hasCommands
                onClicked: row.controller.pardon(row.name)
            }

            ThemedButton {
                visible: row.view === "whitelisted"
                text: qsTr("Remove")
                accent: Theme.danger
                enabled: row.controller.hasCommands
                onClicked: row.controller.unwhitelist(row.name)
            }
        }
    }

    Rectangle {
        anchors.bottom: parent.bottom
        width: parent.width
        height: 1
        color: Theme.rule
    }
}
