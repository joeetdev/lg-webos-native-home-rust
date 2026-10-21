var remoteController = null;
var remoteControls = null;
function init() {
    var imported = Qt.include(__CORE_URL__);
    if (imported.status !== 0) throw new Error('Remote key logic did not load');
    var bridge = Qt.createQmlObject('import QtQuick 2.4; QtObject {property var target: compositor}', root, 'remote-compositor-bridge.qml');
    var file = new XMLHttpRequest();
    file.open('GET', __QML_URL__, false);
    file.send();
    if ((file.status !== 0 && file.status !== 200) || !file.responseText) throw new Error('Remote pointer source unreadable');
    remoteController = Qt.createQmlObject(file.responseText, bridge.target, __QML_URL__);
    if (!remoteController) throw new Error('Native pointer controller did not compile');
    remoteController.power = global;
    remoteController.trialMilliseconds = __TRIAL_MS__;
    remoteController.resetPressed = function() { if (remoteControls) remoteControls.reset(); };
    var cleanup = Qt.createQmlObject('import QtQuick 2.4; QtObject {property var controller:null; Component.onDestruction: {if(controller){controller.enablePointer();controller.destroy();}}}', root, 'remote-cleanup.qml');
    cleanup.controller = remoteController;
    bridge.destroy();
    remoteControls = createRemoteControls({
        active: function() { return global.powerState === 'active'; },
        now: function() { return Date.now(); },
        toggle: function() { remoteController.togglePointer(); },
        movies: function() { applicationManager.checkLaunch('com.crunchyroll.webapp', undefined, false, false); __TRACE_MOVIES__ },
        accepted: KeyPolicy.Accepted, next: KeyPolicy.NextPolicy, device: 3
    });
    remoteController.resetForPower();
    __TRACE_INIT__
}
function handleRemoteControls(key, pressed, autoRepeat, deviceId) {
    __TRACE_KEYS__
    if (!remoteControls) return KeyPolicy.NextPolicy;
    return remoteControls.handle(key, pressed, autoRepeat, deviceId);
}
