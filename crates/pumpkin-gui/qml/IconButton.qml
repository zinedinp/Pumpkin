import QtQuick
import QtQuick.Controls
import org.pumpkin.gui

// A flat, icon-only button.
//
// `icon.color` is what makes the monochrome SVGs follow the theme: the files pin `currentColor`
// to a fixed grey, which Qt's SVG Tiny renderer does not resolve, so Qt Quick Controls recolours
// the rendered image instead.
ToolButton {
    id: button

    property url source
    property color tint: Theme.fg
    property string tooltip: ""

    icon.source: source
    icon.color: enabled ? tint : Theme.fgMuted
    icon.width: 20
    icon.height: 20
    display: AbstractButton.IconOnly
    implicitWidth: 32
    implicitHeight: 30
    opacity: enabled ? 1 : 0.4

    background: Rectangle {
        radius: 4
        color: button.hovered ? Theme.border : "transparent"
    }

    ToolTip.visible: hovered && tooltip !== ""
    ToolTip.text: tooltip
    ToolTip.delay: 400
}
