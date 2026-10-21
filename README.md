# LG webOS Native Home

Personalización reversible del Home nativo de un televisor LG con root. No instala otro launcher: modifica la presentación de `com.webos.app.home`, conserva sus LaunchPoints y acciones originales y añade políticas no visuales dentro del compositor existente.

![Home personalizado](docs/images/native-home.png)

## Estado y alcance

Implementación probada en LG 50UQ8050PSB, webOS 7.6.0 / webOS 22, firmware 04.60.90, arquitectura aarch64 y Homebrew Channel 0.7.3. No se afirma compatibilidad con otros modelos o versiones. Root y acceso administrativo son prerrequisitos; este repositorio no obtiene root ni incluye un exploit.

Se publica el código, las modificaciones reproducibles, las pruebas y la documentación. Se excluyen claves, credenciales, registros privados, ajustes de red, cuentas, respaldos completos, binarios y copias completas del firmware de LG. Los originales necesarios para reconstruir la interfaz deben obtenerse legalmente del propio dispositivo.

## Funciones

- El mismo Home nativo, sin anuncios/categorías ni un dashboard alternativo.
- Paisaje local estático, cuadrícula de seis columnas e iconos de 208 px.
- Lateral: Buscar, TV, HDMI 1/2/3, AirPlay, USB sólo cuando existe almacenamiento real, ThinQ y Ajustes al final.
- Once favoritas y «Todas las apps» como duodécima tarjeta. Catálogo ampliado sin modificar el modelo nativo.
- Orden: Crunchyroll, Netflix, YouTube, Stremio, Prime Video, Moonfin, Disney+, Apple TV, Moonlight, Homebrew Channel y Bili.
- Reloj sin caja: hora/minutos de 152,5 px, segundos de 61 px arriba, color `#ADC4CE` y celdas fijas por dígito. Sólo se actualiza mientras Home está visible.
- Google Assistant alterna el puntero. Apagado por defecto al iniciar/reanudar, sin caducidad tras activarlo manualmente. MOVIES abre Crunchyroll.
- Ocho overrides readonly con validación de hashes, propiedad, inode y montaje; recuperación acotada.
- Un gate no visual evita presentar una ventana Home anterior hasta verificar los overrides y la marca de esa ventana.

TV, entradas, imagen/sonido, CEC/ARC, AirPlay, ThinQ/HomeKit, tienda y motores compartidos quedan fuera del alcance de cualquier eliminación indiscriminada. Ocultar una app no significa desinstalarla. No se incluyen ROMs, BIOS ni datos de aplicaciones.

## Estructura

| Ruta | Contenido |
| --- | --- |
| `home/` | Deltas de presentación, allowlist de firmware y constructor offline; no contiene el firmware completo. |
| `runtime/` | Controlador transaccional de bind mounts, gestión de precarga temporal de Home y recuperación. |
| `gate/` | Proof de ocho archivos, observación de superficies, empaquetado inmutable e instalador con recuperación. |
| `remote/` | Toggle nativo de puntero, remapeo de botones, empaquetado e instalador de ownership independiente. |
| `tools/` | Ejecución de pruebas y controles de publicación sin acceso a la TV. |
| `docs/` | Arquitectura, procedimiento manual, pruebas observadas y límites. |

## Comprobar el repositorio sin una TV

Host recomendado: Python 3.11 o posterior, Node.js 20 o posterior y `sh` en PATH (por ejemplo, Git Bash en Windows, para validar la sintaxis del hook). El runtime destinado a la TV es compatible con Python 3.8; el empaquetado se realiza en el host. No hay dependencias npm/pip para las pruebas offline.

```sh
python tools/test.py
python tools/security_check.py
```

Las pruebas emplean fixtures sintéticos identificados como tales. No contienen ni simulan resultados de pruebas físicas: véase [docs/verification.md](docs/verification.md) para la evidencia y sus límites.

## Reconstruir la interfaz

Conserva fuera del repositorio una copia de los ocho originales QML/JS del firmware indicado, manteniendo la estructura bajo `qml/`. No copies archivos ya cubiertos por un bind mount como si fueran originales. Consulta [home/README.md](home/README.md) para el comando exacto, la validación de hashes y la generación de los ocho overrides.

El fondo es un recurso local externo: usa una imagen propia o con permiso, en la ruta que exige el payload. No se distribuye el archivo de fondo como recurso reutilizable. La captura del README sólo documenta la implementación.

Los productos generados son privados y están ignorados por Git. No ejecutes scripts históricos de otras fases sobre una instalación existente.

## Instalación y recuperación

No existe un instalador universal de un solo comando. [docs/deployment.md](docs/deployment.md) explica los prerrequisitos, el staging manual, el orden de instalación y los puntos de recuperación. Debe revisarse el código antes de ejecutar cualquier operación como root.

El remapeo es independiente de los ocho overrides y de su gate. Los paquetes se pueden generar sin conectar una TV:

```sh
cd remote
python pack-remote.py
# Genera remote-packed-final/package.json y tres assets con nombres SHA256.
```

En la TV, sólo después de revisar compatibilidad, staging, permisos y una recuperación independiente:

```sh
python3 -B -E -s /var/lib/lg-native/remote_controls.py --install /var/lib/lg-native/remote-final-package.json
python3 -B -E -s /var/lib/lg-native/remote_controls.py --check
# Revierte únicamente el remapeo; conserva el gate y filtros ajenos:
python3 -B -E -s /var/lib/lg-native/remote_controls.py --disable
```

`--check` verifica integridad y configuración; no demuestra por sí solo que el mando físico funcione. No sustituyas la cadena completa de filtros por una lista guardada de 17, 18 o 19 entradas.

## Límites importantes

- El bloqueo nativo parece ser una política global de puntero, no un ajuste exclusivo del sensor del Magic Remote. Ratón USB y touchpad del teléfono requieren pruebas separadas.
- Apagado/encendido con el mando, reinicio por software y retirada física de corriente son pruebas distintas.
- Abrir una app no demuestra reproducción de vídeo, DRM, señal HDMI ni casting.
- El gate conserva una salida de recuperación ante fallos de integridad; no promete que el Home original sea imposible de ver bajo cualquier avería.
- El empaquetado y las pruebas offline no demuestran compatibilidad con un firmware diferente.

No se añaden VNC, inyectores de entrada ni procesos diagnósticos residentes. El comportamiento permanente reside en componentes no visuales del compositor ya existente.

## Contribuir

Lee [AGENTS.md](AGENTS.md), utiliza pruebas antes de modificar comportamiento y no publiques datos de tu televisor. Incluye versión exacta del firmware, alcance de la prueba y recuperación, sin credenciales ni dumps completos. Las reglas de redistribución y procedencia se detallan en [NOTICE.md](NOTICE.md).
