import QtQuick 2.4

QtObject {
    id: remote
    property var backend: compositor
    property var power: null
    property var resetPressed: null
    property bool pointerBlocked: false
    property bool desiredEnabled: false
    property int trialMilliseconds: 0
    property Timer restoreTimer: Timer {
        interval: remote.trialMilliseconds > 0 ? remote.trialMilliseconds : 8000
        onTriggered: remote.enablePointer()
    }
    property Connections powerObserver: Connections {
        target: remote.power
        onPowerStateChanged: remote.resetForPower()
    }
    property Connections cursorObserver: Connections {
        target: remote.backend
        onCursorVisibleChanged: remote.cursorVisibilityChanged()
    }
    function cursorVisibilityChanged() {
        if (remote.backend.cursorVisible && !remote.desiredEnabled && remote.power && remote.power.powerState === "active") remote.applyPointerPolicy();
    }
    function applyPointerPolicy() {
        if (!remote.power || remote.power.powerState !== "active") return;
        if (remote.desiredEnabled) {
            remote.backend.cancelBlockPointer();
            remote.pointerBlocked = false;
            remote.restoreTimer.stop();
        } else {
            remote.backend.blockPointer();
            remote.pointerBlocked = true;
            if (remote.trialMilliseconds > 0) remote.restoreTimer.restart();
        }
    }
    function enablePointer() {
        remote.desiredEnabled = true;
        remote.applyPointerPolicy();
    }
    function togglePointer() {
        if (!remote.power || remote.power.powerState !== "active") return;
        remote.desiredEnabled = !remote.desiredEnabled;
        remote.applyPointerPolicy();
    }
    function resetForPower() {
        remote.restoreTimer.stop();
        if (remote.power && remote.power.powerState === "active") {
            remote.applyPointerPolicy();
        } else {
            remote.pointerBlocked = false;
            if (remote.power && remote.power.powerState !== "screen saver") remote.desiredEnabled = false;
        }
        if (typeof remote.resetPressed === "function") remote.resetPressed();
    }
    Component.onDestruction: remote.enablePointer()
}
