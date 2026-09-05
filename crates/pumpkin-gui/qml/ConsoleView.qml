import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.pumpkin.gui

// Log output plus a command line, backed by the same dispatcher as the terminal console.
Item {
    id: view

    required property var controller

    // Bounded so a long-running server cannot grow the model without limit; the Rust ring buffer
    // has its own, larger cap for the backlog.
    readonly property int maxLines: 2000

    property string levelFilter: "all"
    property string search: ""

    readonly property var levelRank: ({
        "trace": 0,
        "debug": 1,
        "info": 2,
        "warn": 3,
        "error": 4
    })

    ListModel {
        id: logModel
    }

    function visibleLine(level, message) {
        if (view.levelFilter !== "all" && levelRank[level] < levelRank[view.levelFilter])
            return false;
        if (view.search !== "" && !message.toLowerCase().includes(view.search.toLowerCase()))
            return false;
        return true;
    }

    function poll() {
        const fresh = view.controller.takeNewLines();
        if (fresh.length === 0)
            return;

        // Whether the view was pinned to the bottom before appending decides whether we follow;
        // checking afterwards would always look "not at the end".
        const following = logList.atYEnd;

        for (let i = 0; i < fresh.length; ++i)
            logModel.append(fresh[i]);

        if (logModel.count > view.maxLines)
            logModel.remove(0, logModel.count - view.maxLines);

        if (following || autoScroll.checked)
            logList.positionViewAtEnd();
    }

    function visibleLogText() {
        const parts = [];
        for (let i = 0; i < logModel.count; ++i) {
            const row = logModel.get(i);
            if (view.visibleLine(row.level, row.message))
                parts.push(row.message);
        }
        return parts.join("\n");
    }

    function defaultLogName() {
        const now = new Date();
        const pad = n => String(n).padStart(2, "0");
        return now.getFullYear()
            + "-" + pad(now.getMonth() + 1)
            + "-" + pad(now.getDate())
            + "_" + pad(now.getHours())
            + "-" + pad(now.getMinutes())
            + "-" + pad(now.getSeconds())
            + ".log";
    }

    function copyVisible() {
        const text = view.visibleLogText();
        if (text === "")
            return;
        clipboardHelper.text = "```\n" + text + "\n```";
        clipboardHelper.selectAll();
        clipboardHelper.copy();
        clipboardHelper.deselect();
        copiedHint.show();
    }

    function openSaveDialog() {
        view.controller.saveLog(view.defaultLogName(), view.visibleLogText());
    }

    function levelColor(level) {
        switch (level) {
        case "error":
            return Theme.danger;
        case "warn":
            return Theme.warn;
        case "debug":
        case "trace":
            return Theme.fgMuted;
        default:
            return Theme.fg;
        }
    }

    Timer {
        interval: 250
        running: true
        repeat: true
        triggeredOnStart: true
        onTriggered: view.poll()
    }

    TextEdit {
        id: clipboardHelper
        visible: false
        width: 0
        height: 0
    }

    Timer {
        id: copiedHint
        interval: 1200
        property bool visible: false
        function show() {
            visible = true;
            restart();
        }
        onTriggered: visible = false
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 8

        RowLayout {
            Layout.fillWidth: true
            spacing: 8

            ThemedComboBox {
                id: levelBox
                Layout.preferredWidth: 130
                model: ["all", "info", "warn", "error"]
                onCurrentTextChanged: view.levelFilter = currentText === "all" ? "all" : currentText
            }

            ThemedField {
                id: searchField
                Layout.fillWidth: true
                Layout.maximumWidth: 260
                placeholderText: qsTr("Filter output…")
                onTextChanged: view.search = text
            }

            Item {
                Layout.fillWidth: true
            }

            ThemedButton {
                text: copiedHint.visible ? qsTr("Copied") : qsTr("Copy")
                enabled: logModel.count > 0
                onClicked: view.copyVisible()
            }

            ThemedButton {
                text: qsTr("Save log")
                enabled: logModel.count > 0
                onClicked: view.openSaveDialog()
            }

            ThemedCheckBox {
                id: autoScroll
                text: qsTr("Follow")
                checked: true
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.fillHeight: true
            color: Theme.surface
            border.color: Theme.border
            border.width: 1
            radius: Theme.radius

            ListView {
                id: logList

                anchors.fill: parent
                anchors.margins: 8
                clip: true
                model: logModel
                boundsBehavior: Flickable.StopAtBounds
                spacing: 1

                ScrollBar.vertical: ScrollBar {}

                // Height lives on the wrapper, not the Text: wrapping Text binds
                // implicitHeight to height and would loop.
                delegate: Item {
                    required property string level
                    required property string message

                    readonly property bool shown: view.visibleLine(level, message)

                    width: ListView.view.width
                    height: shown ? line.implicitHeight : 0
                    visible: shown

                    TextEdit {
                        id: line
                        width: parent.width
                        text: message
                        color: view.levelColor(level)
                        font.family: "monospace"
                        font.pixelSize: 12
                        wrapMode: TextEdit.Wrap
                        textFormat: TextEdit.PlainText
                        readOnly: true
                        selectByMouse: true
                        persistentSelection: false
                    }
                }
            }

            Text {
                anchors.centerIn: parent
                visible: logModel.count === 0
                text: qsTr("No output yet.")
                color: Theme.fgMuted
                font.pixelSize: 12
            }
        }

        CommandInput {
            Layout.fillWidth: true
            controller: view.controller
        }
    }
}
