# Home: deltas reproducibles, sin firmware redistribuido

Este directorio contiene únicamente el constructor offline y datos de reemplazo,
no los ocho archivos completos de LG. Python **3.8 o posterior**, biblioteca
estándar; sin descargas, TV, SSH, comandos privilegiados ni instalación.

## Límite de firmware

`deltas.json` fija ocho pares exactos `sourceSHA256` → `targetSHA256`.
Únicamente se aceptan esos bytes de firmware: coincidir en modelo, versión de
webOS o nombre de archivo **no** basta. No existe modo force ni aplicación
aproximada. Un firmware distinto necesita revisión y nuevos deltas.

Cada edición usa `start` (índice de línea original, base cero), `delete`
(cantidad de líneas originales a omitir) y `lines` (solamente líneas nuevas/de
reemplazo, con terminadores explícitos). No contiene líneas originales eliminadas,
contexto de diff ni cabeceras propietarias. Los offsets siempre corresponden al
original, no al resultado de ediciones anteriores.

## Construcción local

Desde la raíz del repositorio, coloque copias obtenidas legítimamente en un
**directorio privado**, conservando los subdirectorios bajo `qml/` indicados en
`stock` dentro de `home/deltas.json`. No añada esas copias ni resultados al repo.
`private/` y `generated/` son ejemplos de rutas relativas privadas/ignoradas:

```sh
python home/build_home.py --stock-dir private/stock-qml --out private/generated-home
python -m unittest discover -s home/tests -v
```

El padre de `--out` debe existir; la carpeta de salida debe ser nueva y estar
fuera del árbol stock. No se sobrescriben salidas. Se rechazan archivos ausentes,
no regulares, symlinks/reparse points y componentes de ruta inseguros. Todos los
hashes de entrada se comprueban **antes de transformar nada**; todos los hashes
de salida se comprueban antes de crear la salida. Use directorios locales bajo
su control, sin procesos que los modifiquen durante la operación.

No hay normalización silenciosa. Si una copia local fue convertida a CRLF,
únicamente este modo explícito convierte CRLF → LF **en memoria**, antes de
validar los hashes; no modifica los originales, no elimina BOM y no convierte
CR aislado:

```sh
python home/build_home.py --stock-dir private/stock-qml --out private/generated-home-lf --normalize-crlf
```

Se producen ocho `native-*` y `home-final-manifest.json`. El manifest conserva
`source`, `target`, `originalHash`, `newHash`, compatible con el subsistema
`runtime/`. Para preparación **manual**, `source` usa `R=/var/lib/lg-native`:
`/var/lib/lg-native/native-MainView.qml`, etc. `target` es el destino original
bajo `/usr/palm/applications/com.webos.app.home/qml/`. Estas rutas de dispositivo
son constantes públicas; nunca se escriben rutas privadas del equipo autor.
El constructor no copia nada al dispositivo ni autoriza una instalación.

## Fondo necesario, no distribuido

La reproducción exacta de `native-MainView.qml` referencia:

```text
file:///var/lib/lg-native/home-assets/sidebar-landscape-92d63fb128d8b806b6234b2d9a8034f55c4a1fc406f031192de8a5494d21f529.png
```

La imagen **no está incluida**, y su licencia/permisos de redistribución no
están establecidos aquí. Debe obtenerla y preparar ese recurso por separado con
los derechos correspondientes. El constructor no comprueba ni instala la
imagen. Sin ella puede faltar el fondo aunque los ocho hashes sean correctos.
Cambiar la referencia implica un payload distinto, no reproducción byte a byte.

## Qué está comprobado

`verification.json` registra una ejecución real contra stock privado y el
manifest final independiente: **8/8 hashes de entrada, 8/8 hashes finales y 8/8
comparaciones byte a byte con el candidato**. La entrada CRLF falló en modo
estricto y pasó solamente con `--normalize-crlf`. Las pruebas unitarias usan
fixtures sintéticos, nunca archivos stock. Esto no equivale a compilar QML con
las importaciones privadas de LG ni a validar foco, mando, arranque o standby
en un televisor. Los resultados generados contienen firmware y deben seguir
siendo privados; publicar deltas no concede derechos sobre fuentes o imágenes
ajenas.
