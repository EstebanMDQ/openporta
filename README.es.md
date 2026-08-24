# openporta

*[English](README.md) · **Español***

Una emulación por software de un portaestudio de casete de 4 pistas.

Cuatro pistas mono, un conjunto fijo de controles y un flujo de trabajo
destructivo. Grabás encima de las cosas. Mezclás tres pistas a una para
liberarlas, y esa mezcla te cuesta una generación. La limitación es el
punto: esto es un instrumento, no una DAW.

## Qué hace

- **Cuatro pistas mono, un master estéreo.** Nunca más que eso.
- **Grabación destructiva.** Grabar sobre una pista la borra. Mezclar
  imprime la mezcla en un bus de mezcla estéreo dedicado, en tiempo
  real, así que se pueden mover faders y paneos mientras se graba.
- **Pérdida de generación real.** El carácter de cinta se imprime al
  momento de grabar, así que cada mezcla vuelve a saturar, opacar y
  hacer fluctuar el material, y el piso de ruido sube. Tres
  generaciones suenan a tres generaciones.
- **Silencio (mute) y monitor de entrada por pista**, independientes
  del armado: revisá un nivel o escuchá un micrófono antes de
  comprometerte, silenciá una pista sin tocar su fader.
- **Deshacer igual.** La sensación destructiva es real, pero un
  historial oculto guarda cada pasada de grabación, así que un error se
  puede recuperar. No hay navegador de historial; solo deshacer y
  rehacer.
- **Carácter de casete**: saturación de cinta, un techo en 11kHz, wow y
  flutter que se descorrelacionan entre pasadas, y siseo impreso dentro
  de la banda de paso para que se acumule como el siseo real. La
  reducción de bits (bitcrush) está disponible y desactivada por
  defecto.

Deliberadamente fuera de alcance: MIDI, sincronización por red,
plugins, cantidad variable de pistas, edición no destructiva. Mirá
`openspec/spec.md`.

## Capturas

Corriendo en vivo sobre el objetivo de despliegue: una Raspberry Pi 4
con una interfaz Zoom L6, en modo kiosco:

<img src="docs/screenshots/mixer.png" width="480" alt="La vista del mezclador: cuatro tiras de pista con armar/silenciar/monitor, faders verticales, medidores, master, transporte y una sesión Zoom L6 conectada.">

Tiras de pista (armar/silenciar/monitor, fader y medidor verticales,
paneo), la barra de posición de cinta debajo del contador y una
conexión de dispositivo en vivo: sin mouse, esto corre a pantalla
completa pensado para una pantalla táctil. La vista de Cintas (selector
de casetes, indicador de espacio libre, exportación) y la de Ajustes
(selección de dispositivo, interruptor de kiosco) están detrás de dos
botones en esta misma pantalla, fuera de ella por defecto para que el
mezclador entre en una pantalla de kiosco de 800x480 sin scroll.

<img src="docs/screenshots/desktop-icon.png" width="480" alt="El escritorio de la Raspberry Pi con un ícono lanzador de openporta con forma de casete.">

También vive como una aplicación de escritorio común - lanzador en la
barra de tareas, ícono de escritorio - para quien quiera iniciarla
manualmente en lugar de arrancar directo en modo kiosco.

## Estado

El motor está completo y probado sin interfaz. La interfaz Slint lo
maneja a través de la cola de comandos; con la función `realtime`
activada y un dispositivo conectado, es un camino de audio cpal real de
punta a punta, verificado contra una Zoom L6 tanto en macOS como en una
Raspberry Pi 4.

| Hito | Estado |
|------|--------|
| M0 andamiaje, CI, instrumentos de prueba | hecho |
| M1 motor de cinta: transporte, grabación, pinchado, deshacer, persistencia | hecho |
| M2 DSP lo-fi y pérdida de generación | hecho |
| M3 mezcla, mixdown, exportación WAV, CLI | hecho |
| M4 audio en tiempo real (cpal) | verificado en macOS y en hardware Pi |
| M5 interfaz Slint: transporte, tiras de pista, medidores, barra de posición, guardar/deshacer, vista de Cintas, exportación, audio real | hecho |
| M7 bus de mezcla estéreo (cambio 001) | hecho, publicado en v0.1.0 |
| M6 despliegue en Raspberry Pi | en curso: compilación aarch64, capa de dispositivos ALSA/PipeWire, grabación/guardado full-duplex, reconexión automática al dispositivo recordado, autoarranque en kiosco con íconos, y autoguardado al detener, todo verificado en hardware real; falta el perfilado de rendimiento (M6.2) |

Se encontraron y corrigieron acá tres errores distintos de seguridad en
tiempo real, ninguno por una falla: grabar reservaba un buffer del
tamaño de toda la cinta en el hilo del callback de audio; un camino de
expulsión soltaba sus bloques pre-reservados al heap en lugar de
devolverlos; y enganchar la grabación reconstruía la cadena de DSP
entera - cuatro o cinco reservas - cada vez, sin que nadie lo notara
por meses. Cada uno lo atrapó una revisión adversarial y se corrigió
con una prueba de regresión.

La propuesta de un bus de mezcla estéreo dedicado
(`openspec/changes/001-stereo-repeatable-bounce.md`) fue **aprobada
tras doce rondas de revisión a lo largo de trece revisiones**, y todas
las rondas menos la última encontraron un error o un hueco real. Ya
está incorporada a `openspec/spec.md` (v1.1) y completamente
implementada: impresión estéreo en tiempo real, deshacer atómico de dos
canales, mezclas que se pliegan hacia adelante en vez de reemplazarse,
el fader master demostrablemente sin llegar nunca a la cinta, y una
tira de Bus en la interfaz con su propio fader y silencio.

REQ-902 se mide en vez de argumentarse: un asignador global contador,
solo para pruebas, verifica **cero reservas y cero liberaciones** en
`record -> process_block -> stop`, tanto para una pasada de pista como
para una mezcla. La primera vez que corrió encontró cuatro violaciones
más que el razonamiento cuidadoso había pasado por alto.

Hoy lo manejás con guiones de sesión, la CLI o la interfaz. `TASKS.md`
es la cola de trabajo.

## Probalo

```bash
# crear un casete y grabar algo en la pista 1
cargo run -p porta-app -- new mitape.porta --minutes 5
cargo run -p porta-app -- script sesion.json
cargo run -p porta-app -- render mitape.porta --out mezcla.wav --bits 24
```

Un guion de sesión es una lista de operaciones:

```json
{"ops": [
  {"op": "new", "dir": "mitape.porta", "minutes": 5, "seed": 1979},
  {"op": "arm", "track": 0},
  {"op": "record", "input_wav": "guitarra.wav"},
  {"op": "arm", "track": 0, "on": false},
  {"op": "fader", "track": 0, "db": -3.0},
  {"op": "pan", "track": 0, "value": -0.4},
  {"op": "bounce_arm"},
  {"op": "seek", "seconds": 0},
  {"op": "bounce", "seconds": 30},
  {"op": "bounce_arm", "on": false},
  {"op": "seek", "seconds": 0},
  {"op": "export", "out": "descartar.wav"},
  {"op": "play", "seconds": 30},
  {"op": "export", "out": "mezcla.wav"},
  {"op": "save"}
]}
```

`export` escribe todo lo que la máquina reprodujo desde el export
anterior, y por eso el ejemplo descarta uno antes de la toma que
realmente quiere. `character` en la operación `new` acepta `cassette`
(por defecto) o `clean`, útil cuando querés la mecánica sin el color.

Con hardware de audio real:

```bash
cargo run -p porta-app --features realtime -- devices
cargo run -p porta-app --features realtime -- live mitape.porta --period 256

# elegir qué canal de entrada alimenta cada pista (base 1, como los que
# imprime `probe`; "-" deja una pista en silencio)
cargo run -p porta-app --features realtime -- live mitape.porta --in-map 3,4,5,6
```

La interfaz Slint, con audio real si la función `realtime` también está
activada:

```bash
cargo run -p porta-app --features ui,realtime -- ui mitape.porta
# --kiosk corre a pantalla completa y sin marco, para una pantalla
# táctil dedicada. Escape lo desactiva, o usá el mismo interruptor
# desde la vista de Ajustes
cargo run -p porta-app --features ui,realtime -- ui mitape.porta --kiosk
```

La interfaz recuerda el último dispositivo de audio que se conectó
correctamente y se reconecta automáticamente al iniciar: está pensada
para comportarse como un aparato listo apenas se enciende, no como una
herramienta que arranca inactiva. `docs/pi-setup.md` cubre el
autoarranque en kiosco, el lanzador de la barra de tareas y el ícono de
escritorio específicamente en la Pi.

## Versiones publicadas

Las versiones etiquetadas (`vX.Y.Z`) compilan `porta-app` para macOS
(Apple Silicon e Intel), Linux (x86_64 y aarch64) y Windows, con las
funciones `realtime` y `ui` activadas. Mirá la pestaña Actions, o
`.github/workflows/release.yml`.

## Distribución

```
crates/porta-dsp/      carácter de cinta: saturación, ancho de banda, flutter, siseo
crates/porta-engine/   cinta, transporte, pasadas de grabación, deshacer, mezclador, proyectos
crates/porta-testkit/  instrumentos de prueba: generadores, medidores, FFT, detector de clics
crates/porta-app/      CLI, guiones de sesión, exportación WAV, adaptador de tiempo real, interfaz Slint
openspec/spec.md       los requisitos establecidos
openspec/changes/      propuestas para cambiar requisitos establecidos, en revisión
docs/manual-checklist.md  lo que solo puede verificar una persona con hardware
docs/pi-setup.md       autoarranque en kiosco, lanzador, ícono de escritorio
```

El motor no sabe nada del hardware de audio: buffers que entran,
buffers que salen. Eso es lo que permite probar todo sin placa de
sonido, y lo que permite que corra en una Pi sin que el motor se entere.

## Desarrollo

```bash
cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

En una máquina sin toolchain de Rust, `scripts/cargo-docker.sh` corre
los mismos comandos en un contenedor.

La corrección del audio se verifica renderizando sin tiempo real y
midiendo: ventanas RMS, energía por banda, distorsión armónica total,
desviación de altura en cents y un detector de clics que atrapa
discontinuidades que ningún oyente está presente para escuchar. Un
render "golden" fija el sonido exacto de una sesión completa; si
cambia, algo cambió, y la razón va en `TASKS.md` antes de bendecirlo.

## Un casete en disco

```
mitape.porta/
  manifest.json         largo de cinta, carácter y semilla, ajustes del mezclador
  tape/track{0..3}.raw  muestras crudas de 16 bits, guardadas en bloques de 5 segundos
  tape/bounce_{l,r}.raw el bus de mezcla estéreo, mismo formato por bloques
  undo/                 el historial que hace posible deshacer
```

Los guardados reescriben solo los bloques que cambiaron, y nunca
ocurren mientras la cinta está rodando.
