/* Imported as plain JavaScript by Qt5 QML; no Node or device APIs. */
function nativeGateProof(readText) {
    var root = "/var/lib/lg-native/";
    var home = "/usr/palm/applications/com.webos.app.home/qml/";
    var targets = [
        "UserInterfaceLayer/Containers/MainView.qml",
        "UserInterfaceLayer/Containers/Main/MainRibbon.qml",
        "UserInterfaceLayer/Containers/Main/TopShelf.qml",
        "Model/ShelfList.js", "Configurations/SystemServiceResponseProcessors.js",
        "Interfaces/ConnectionsInterfaces/ConnectionsInterfaces.qml",
        "UserInterfaceLayer/Containers/Main/HomeDashboard.qml",
        "UserInterfaceLayer/Containers/Sub/MobileComponent.qml"
    ].map(function (target) { return home + target; });
    function sourcePath(value) {
        return typeof value === "string" && value.indexOf(root) === 0 && value.length > root.length &&
            value.charAt(value.length - 1) !== "/" && value.indexOf("//") < 0 &&
            !/(^|\/)\.{1,2}(\/|$)/.test(value) && !/[\\?#%:\s\x00-\x1f\x7f]/.test(value);
    }
    try {
        var manifestText = readText(root + "home-final-manifest.json");
        var journalText = readText(root + "home-mounts.json");
        if (typeof manifestText !== "string" || typeof journalText !== "string") return false;
        var manifest = JSON.parse(manifestText);
        var journal = JSON.parse(journalText);
        if (!Array.isArray(manifest) || manifest.length !== 8 ||
                !Array.isArray(journal.mounts) || journal.mounts.length !== 8 ||
                journal.owner !== "lg-native-home-v1" || journal.version !== 1 ||
                !journal.preload || journal.preload.pending !== false ||
                typeof journal.preload.original !== "boolean" ||
                typeof journal.preload.bootId !== "string" || !journal.preload.bootId) return false;
        var owned = {}, seen = [], inputs = [], copies = [], ids = [], i, entry, mount;
        for (i = 0; i < 8; i++) {
            mount = journal.mounts[i];
            if (!mount || targets.indexOf(mount.target) < 0 || owned[mount.target] ||
                    !sourcePath(mount.source) || copies.indexOf(mount.source) >= 0 ||
                    typeof mount.mountId !== "number" || mount.mountId <= 0 ||
                    mount.mountId > 9007199254740991 || Math.floor(mount.mountId) !== mount.mountId ||
                    ids.indexOf(mount.mountId) >= 0 || mount.uid !== 0 || mount.gid !== 0 || mount.mode !== 493 ||
                    mount.state !== "mounted" || mount.bootId !== journal.preload.bootId) return false;
            copies.push(mount.source);
            ids.push(mount.mountId);
            owned[mount.target] = mount;
        }
        for (i = 0; i < 8; i++) {
            entry = manifest[i];
            if (!entry || targets.indexOf(entry.target) < 0 || seen.indexOf(entry.target) >= 0 ||
                    !sourcePath(entry.source) || inputs.indexOf(entry.source) >= 0 ||
                    typeof entry.newHash !== "string" || !/^[a-f0-9]{64}$/.test(entry.newHash) ||
                    typeof entry.originalHash !== "string" || !/^[a-f0-9]{64}$/.test(entry.originalHash) ||
                    entry.newHash === entry.originalHash) return false;
            seen.push(entry.target);
            inputs.push(entry.source);
            mount = owned[entry.target];
            if (mount.inputSource !== entry.source || mount.newHash !== entry.newHash ||
                    mount.originalHash !== entry.originalHash) return false;
            var text = readText(entry.source);
            if (typeof text !== "string" || text.length === 0 ||
                    readText(entry.target) !== text || readText(mount.source) !== text) return false;
        }
        return readText(root + "home-final-manifest.json") === manifestText &&
            readText(root + "home-mounts.json") === journalText;
    } catch (error) {
        return false;
    }
}
