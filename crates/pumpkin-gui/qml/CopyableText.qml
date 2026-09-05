import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.pumpkin.gui

// A monospace value with a copy button, for things you actually need on the clipboard.
//
// Qt Quick exposes no clipboard type to QML and cxx-qt-lib has no QClipboard binding, so the copy
// goes through an off-screen TextEdit -- `copy()` works on the document, not on what is painted.
RowLayout {
    id: field

    property string value: ""
    property string label: ""

    spacing: 6

    ColumnLayout {
        spacing: 0

        Text {
            text: field.value
            color: Theme.fg
            font.pixelSize: 13
            font.family: "monospace"
        }

        Text {
            text: field.label
            visible: field.label !== ""
            color: Theme.fgMuted
            font.pixelSize: 11
        }
    }

    TextEdit {
        id: clipboardHelper
        visible: false
        text: field.value
    }

    ToolButton {
        id: copyButton

        implicitWidth: 24
        implicitHeight: 24
        Layout.alignment: Qt.AlignTop

        onClicked: {
            clipboardHelper.selectAll();
            clipboardHelper.copy();
            clipboardHelper.deselect();
            copiedHint.show();
        }

        contentItem: Text {
            // Two offset rectangles: the usual "copy" glyph, drawn with text so it needs no asset.
            text: copiedHint.visible ? "✓" : "⧉"
            color: copiedHint.visible ? Theme.good : Theme.fgMuted
            font.pixelSize: 13
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
        }

        background: Rectangle {
            radius: 4
            color: copyButton.hovered ? Theme.surfaceAlt : "transparent"
        }

        ToolTip.visible: hovered && !copiedHint.visible
        ToolTip.text: qsTr("Copy")
        ToolTip.delay: 400
    }

    Timer {
        id: copiedHint

        interval: 1200

        function show() {
            visible = true;
            restart();
        }

        property bool visible: false
        onTriggered: visible = false
    }
}
