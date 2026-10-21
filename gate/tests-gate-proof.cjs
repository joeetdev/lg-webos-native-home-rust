'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const test = require('node:test');
const crypto = require('node:crypto');
const ROOT = '/var/lib/lg-native/';
const MANIFEST = ROOT + 'home-final-manifest.json';
const JOURNAL = ROOT + 'home-mounts.json';
const baselineManifest = JSON.parse(fs.readFileSync(path.join(__dirname, 'fixtures/home-final-manifest.json'), 'utf8'));
const baselineJournal = JSON.parse(fs.readFileSync(path.join(__dirname, 'fixtures/home-mounts.json'), 'utf8'));
const context = vm.createContext({});
const productionFile = path.join(__dirname, 'gate-proof.js');
if (fs.existsSync(productionFile)) vm.runInContext(fs.readFileSync(productionFile, 'utf8'), context);
function prove(readText) {
    assert.equal(typeof context.nativeGateProof, 'function', 'nativeGateProof is not implemented');
    return context.nativeGateProof(readText);
}
function fixture() {
    const manifest = structuredClone(baselineManifest);
    const journal = structuredClone(baselineJournal);
    const files = Object.create(null);
    manifest.forEach((entry, i) => {
        const text = fs.readFileSync(path.join(__dirname, 'fixtures', path.posix.basename(entry.source)), 'utf8').replace(/\r\n/g, '\n');
        assert.equal(crypto.createHash('sha256').update(text).digest('hex'), entry.newHash, 'synthetic source must match fixture hash');
        files[entry.source] = files[entry.target] = files[journal.mounts[i].source] = text;
    });
    const calls = [];
    function sync() {
        files[MANIFEST] = JSON.stringify(manifest);
        files[JOURNAL] = JSON.stringify(journal);
    }
    sync();
    function readText(file) {
        calls.push(file);
        if (!Object.hasOwn(files, file)) throw new Error('Missing file: ' + file);
        return files[file];
    }
    return { manifest, journal, files, calls, sync, readText };
}

test('synthetic eight-file baseline transitions from absent root evidence to ready', () => {
    const f = fixture();
    assert.equal(prove(() => { throw new Error('root has not run'); }), false);
    assert.equal(prove(f.readText), true);
});

function reject(name, mutate) {
    test(name, () => {
        const f = fixture();
        mutate(f);
        f.sync();
        assert.equal(prove(f.readText), false);
    });
}
for (const [field, value] of [['owner', 'foreign'], ['owner', undefined], ['version', 2], ['version', '1']]) {
    reject(`journal ${field}=${String(value)} rejected`, f => { f.journal[field] = value; });
}
for (const value of [undefined, null, {}, { original: true, pending: true, bootId: baselineJournal.preload.bootId },
    { original: 'true', pending: false, bootId: baselineJournal.preload.bootId },
    { original: true, pending: 0, bootId: baselineJournal.preload.bootId },
    { original: true, pending: false, bootId: '' }, { original: true, pending: false, bootId: 123 }]) {
    reject(`preload completion record ${JSON.stringify(value)} rejected`, f => { f.journal.preload = value; });
}
for (let i = 0; i < 8; i++) {
    for (const value of [undefined, 'copy', 'intent', 'removed', true]) {
        reject(`mount ${i} state ${String(value)} rejected`, f => { f.journal.mounts[i].state = value; });
    }
    for (const value of [undefined, '', 'different-boot', 123]) {
        reject(`mount ${i} boot ${String(value)} rejected`, f => { f.journal.mounts[i].bootId = value; });
    }
}
test('restored original preload false is accepted', () => {
    const f = fixture();
    f.journal.preload.original = false;
    f.sync();
    assert.equal(prove(f.readText), true);
});

test('ownership entries match by target, not journal order', () => {
    const f = fixture();
    f.journal.mounts.reverse();
    f.manifest.reverse();
    f.manifest.push(f.manifest.shift());
    f.sync();
    assert.equal(prove(f.readText), true);
});
for (const side of ['manifest', 'mounts']) {
    for (const fault of ['short', 'long', 'duplicate', 'arraylike']) {
        reject(`${side} ${fault} rejected`, f => {
            const entries = side === 'manifest' ? f.manifest : f.journal.mounts;
            if (fault === 'short') entries.pop();
            if (fault === 'long') entries.push(structuredClone(entries[0]));
            if (fault === 'duplicate') entries[7] = structuredClone(entries[0]);
            if (fault === 'arraylike') {
                const value = Object.assign({ length: 8 }, entries);
                if (side === 'manifest') f.files[MANIFEST] = JSON.stringify(value);
                else f.journal.mounts = value;
                if (side === 'manifest') f.sync = () => { f.files[JOURNAL] = JSON.stringify(f.journal); };
            }
        });
    }
}
for (let i = 0; i < 8; i++) {
    for (const side of ['manifest', 'mounts']) {
        for (const value of [null, {}, 'bad']) {
            reject(`${side} entry ${i} ${JSON.stringify(value)} rejected`, f => {
                (side === 'manifest' ? f.manifest : f.journal.mounts)[i] = value;
            });
        }
    }
    reject(`foreign target ${i} rejected even with matching text`, f => {
        const foreign = '/usr/palm/applications/com.webos.app.home/qml/Foreign' + i + '.qml';
        f.files[foreign] = f.files[f.manifest[i].target];
        f.manifest[i].target = f.journal.mounts[i].target = foreign;
    });
    reject(`journal target ${i} mismatch rejected`, f => { f.journal.mounts[i].target += '.different'; });
}

for (let i = 0; i < 8; i++) {
    for (const field of ['newHash', 'originalHash']) {
        for (const value of [undefined, '', 'a'.repeat(63), 'g'.repeat(64), 'A'.repeat(64), 123]) {
            reject(`manifest ${i} invalid ${field} ${String(value)} rejected`, f => {
                f.manifest[i][field] = f.journal.mounts[i][field] = value;
            });
        }
        reject(`journal ${i} mismatched ${field} rejected`, f => { f.journal.mounts[i][field] = '0'.repeat(64); });
    }
    reject(`unchanged stock hash ${i} rejected`, f => {
        f.manifest[i].newHash = f.journal.mounts[i].newHash = f.manifest[i].originalHash;
    });
    reject(`journal ${i} inputSource mismatch rejected even if readable`, f => {
        f.journal.mounts[i].inputSource = '/var/lib/lg-native/other/source.qml';
        f.files[f.journal.mounts[i].inputSource] = f.files[f.manifest[i].source];
    });
}
reject('duplicate manifest input source rejected despite equal payloads', f => {
    f.manifest[7].source = f.journal.mounts[7].inputSource = f.manifest[0].source;
    f.manifest[7].newHash = f.journal.mounts[7].newHash = f.manifest[0].newHash;
    f.files[f.manifest[7].target] = f.files[f.journal.mounts[7].source] = f.files[f.manifest[0].source];
});
reject('duplicate journal backing source rejected despite equal payloads', f => {
    f.journal.mounts[7].source = f.journal.mounts[0].source;
    f.manifest[7].newHash = f.journal.mounts[7].newHash = f.manifest[0].newHash;
    f.files[f.manifest[7].target] = f.files[f.manifest[7].source] = f.files[f.manifest[0].source];
});

const invalidSources = [undefined, null, 123, '', 'relative.qml', '/tmp/payload.qml',
    '/var/lib/lg-native-evil/payload.qml', '/var/lib/lg-native/', '/var/lib/lg-native',
    '/var/lib/lg-native//payload.qml', '/var/lib/lg-native/../payload.qml',
    '/var/lib/lg-native/./payload.qml', '/var/lib/lg-native/x/../../payload.qml',
    '/var/lib/lg-native/x/..', '/var/lib/lg-native/x/.', '/var/lib/lg-native/payload.qml/',
    '/var/lib/lg-native/x\\payload.qml', '/var/lib/lg-native/payload.qml?x=y',
    '/var/lib/lg-native/payload.qml#fragment', '/var/lib/lg-native/%2e%2e/payload.qml',
    'file:///var/lib/lg-native/payload.qml', '/var/lib/lg-native/file:\u0000.qml'];
for (let i = 0; i < 8; i++) {
    for (const role of ['input', 'copy']) {
        for (const value of invalidSources) {
            reject(`${role} ${i} invalid path ${JSON.stringify(value)} rejected with readable equal text`, f => {
                const old = role === 'input' ? f.manifest[i].source : f.journal.mounts[i].source;
                f.files[value] = f.files[old];
                if (role === 'input') f.manifest[i].source = f.journal.mounts[i].inputSource = value;
                else f.journal.mounts[i].source = value;
            });
        }
    }
}
test('canonical root source paths do not depend on a release directory name', () => {
    const f = fixture();
    f.manifest.forEach((entry, i) => {
        const next = ROOT + 'another-release-9000/' + path.posix.basename(entry.source);
        f.files[next] = f.files[entry.source];
        delete f.files[entry.source];
        entry.source = f.journal.mounts[i].inputSource = next;
    });
    f.sync();
    assert.equal(prove(f.readText), true);
});
for (const field of ['newHash', 'originalHash']) {
    reject(`${field} rejects final newline rather than matching a prefix`, f => {
        f.manifest[0][field] += '\n';
        f.journal.mounts[0][field] = f.manifest[0][field];
    });
}

for (let i = 0; i < 8; i++) {
    for (const value of [undefined, null, 0, -1, 1.5, '501', true, Number.MAX_SAFE_INTEGER + 1]) {
        reject(`mount ${i} invalid mountId ${String(value)} rejected`, f => { f.journal.mounts[i].mountId = value; });
    }
    reject(`mount ${i} duplicate mountId rejected`, f => { f.journal.mounts[i].mountId = f.journal.mounts[(i + 1) % 8].mountId; });
    for (const field of ['uid', 'gid', 'mode']) {
        for (const value of [undefined, null, true, '0', -1, field === 'mode' ? 384 : 1]) {
            reject(`mount ${i} invalid recorded ${field} ${String(value)} rejected`, f => { f.journal.mounts[i][field] = value; });
        }
    }
}

for (const file of [MANIFEST, JOURNAL]) {
    for (const fault of ['missing', 'empty', 'json', 'truncated', 'coercible-object']) {
        test(`${path.posix.basename(file)} ${fault} rejected`, () => {
            const f = fixture();
            const saved = f.files[file];
            if (fault === 'missing') delete f.files[file];
            if (fault === 'empty') f.files[file] = '';
            if (fault === 'json') f.files[file] = '{broken';
            if (fault === 'truncated') f.files[file] = saved.slice(0, -1);
            if (fault === 'coercible-object') f.files[file] = { toString: () => saved };
            assert.equal(prove(f.readText), false);
        });
    }
    for (const fault of ['changed', 'removed', 'throws']) {
        test(`${path.posix.basename(file)} final reread ${fault} fails closed`, () => {
            const f = fixture();
            let n = 0;
            assert.equal(prove(name => {
                if (name === file && ++n === 2) {
                    if (fault === 'throws') throw new Error('evidence replaced');
                    return fault === 'removed' ? undefined : f.readText(name) + '\n';
                }
                return f.readText(name);
            }), false);
            assert.equal(n, 2, 'must recheck exact evidence after payload verification');
        });
    }
}
test('metadata rereads follow all 24 complete payload reads', () => {
    const f = fixture();
    assert.equal(prove(f.readText), true);
    assert.deepEqual(f.calls.slice(0, 2), [MANIFEST, JOURNAL]);
    assert.deepEqual(f.calls.slice(-2), [MANIFEST, JOURNAL]);
    assert.deepEqual(f.calls.slice(2, -2).sort(), f.manifest.flatMap(entry => [entry.source, entry.target,
        f.journal.mounts.find(mount => mount.target === entry.target).source]).sort());
});
for (let failureAt = 0; failureAt < 28; failureAt++) {
    test(`read error at boundary ${failureAt} fails closed`, () => {
        const f = fixture();
        let n = 0;
        assert.equal(prove(file => {
            if (n++ === failureAt) throw new Error('read interrupted');
            return f.readText(file);
        }), false);
    });
}

test('ready is recomputed and never latched across pending or missing source', () => {
    const f = fixture();
    assert.equal(prove(f.readText), true);
    f.journal.preload.pending = true;
    f.sync();
    assert.equal(prove(f.readText), false);
    f.journal.preload.pending = false;
    f.sync();
    assert.equal(prove(f.readText), true);
    delete f.files[f.manifest[0].source];
    assert.equal(prove(f.readText), false);
});
test('plain QML-script context exposes only nativeGateProof without host APIs', () => {
    assert.deepEqual(Object.keys(context), ['nativeGateProof']);
    const source = fs.readFileSync(productionFile, 'utf8');
    assert.doesNotMatch(source, /=>|`|\b(?:let|const|class|require|module|process|Buffer|Promise|fetch|XMLHttpRequest)\b/);
    const f = fixture();
    const isolated = vm.createContext({ readText: f.readText });
    vm.runInContext(source, isolated);
    assert.equal(vm.runInContext('nativeGateProof(readText)', isolated), true);
});

for (let i = 0; i < 8; i++) {
    for (const role of ['input', 'target', 'copy']) {
        for (const fault of ['missing', 'empty', 'nonstring', 'stock', 'prefix']) {
            test(`file ${i} ${role}: ${fault} fails closed`, () => {
                const f = fixture();
                const file = role === 'input' ? f.manifest[i].source : role === 'target' ? f.manifest[i].target : f.journal.mounts[i].source;
                if (fault === 'missing') delete f.files[file];
                if (fault === 'empty') f.files[file] = '';
                if (fault === 'nonstring') f.files[file] = {};
                if (fault === 'stock') f.files[file] = 'stock QML, not the current root payload';
                if (fault === 'prefix') f.files[file] = f.files[file].slice(0, Math.min(16384, f.files[file].length - 1));
                assert.equal(prove(f.readText), false);
            });
        }
    }
}
