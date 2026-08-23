---
title: Primeros pasos
description: Cómo correr openporta desde la línea de comandos, guiones de sesión y dónde conseguir una compilación.
date: 2026-08-22
lang: es
---

*[English](../getting-started.html) · **Español***

# Primeros pasos

openporta es un workspace de Rust. No hay instalador: compilalo, o
bajate una compilación publicada, y apuntalo a un directorio que
contenga un casete.

## Las tres formas de manejarlo

**Sin interfaz gráfica**, mediante un guion de sesión, que es como
funciona cada prueba del proyecto:

```bash
cargo run -p porta-app -- new mitape.porta --minutes 5
cargo run -p porta-app -- script sesion.json
cargo run -p porta-app -- render mitape.porta --out mezcla.wav --bits 24
```

Un guion de sesión es una lista JSON de operaciones:

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

Mezclar (bounce) es armar el bus y hacer rodar el transporte, no un
comando por lotes: la mezcla se imprime en tiempo real sobre un bus de
mezcla estéreo dedicado, así que se pueden mover faders y paneos
mientras se graba, y el bus conserva su contenido anterior: si volvés a
mezclar, la generación previa se pliega hacia adelante en vez de ser
reemplazada.

`export` escribe todo lo que la máquina reprodujo desde el export
*anterior*, y por eso el ejemplo de arriba descarta uno antes de la
toma que realmente quiere: la cinta tiene que pasar por el material una
vez para que haya algo nuevo que capturar. El campo `character` de la
operación `new` acepta `cassette` (la formulación lo-fi por defecto) o
`clean`, útil cuando querés el transporte y la mecánica de grabación
sin el color.

**Con hardware de audio real**, mediante cpal:

```bash
cargo run -p porta-app --features realtime -- devices
cargo run -p porta-app --features realtime -- live mitape.porta --period 256
```

`devices` lista lo que está realmente disponible y con qué nombre
dirigirse a cada cosa; conviene revisarlo primero, porque algunos
backends de audio enumeran la misma interfaz física muchas veces con
nombres casi idénticos.

Para elegir qué canal de entrada alimenta cada pista, usá `--in-map`
con números de canal de base 1, los mismos que imprime `probe`
(`-` deja una pista en silencio):

```bash
cargo run -p porta-app --features realtime -- probe --in "L6"
cargo run -p porta-app --features realtime -- live mitape.porta --in-map 3,4,5,6
```

**Por la interfaz gráfica**, con audio real si la función `realtime`
también está activada:

```bash
cargo run -p porta-app --features ui,realtime -- ui mitape.porta

# --kiosk corre a pantalla completa y sin marco, para una pantalla
# táctil dedicada. Escape lo vuelve a desactivar, o usá el mismo
# interruptor desde Ajustes
cargo run -p porta-app --features ui,realtime -- ui mitape.porta --kiosk
```

La interfaz recuerda el último dispositivo de audio que conectó
correctamente y se reconecta a él automáticamente al iniciar: está
hecha para comportarse como un aparato que está listo apenas se
enciende, no como una herramienta que arranca inactiva esperando
configuración. Mirá [Configuración de Raspberry
Pi](raspberry-pi.html) para la historia completa del modo kiosco.

## Compilaciones publicadas

Las versiones etiquetadas (`vX.Y.Z`) se compilan para macOS (Apple
Silicon e Intel), Linux (x86_64 y aarch64) y Windows, con las funciones
`realtime` y `ui` activadas. Mirá la pestaña Actions del proyecto para
ver la matriz de compilación.

## Un casete en disco

```
mitape.porta/
  manifest.json         largo de cinta, carácter y semilla, ajustes del mezclador
  tape/track{0..3}.raw  muestras crudas de 16 bits, guardadas en bloques de 5 segundos
  tape/bounce_{l,r}.raw el bus de mezcla estéreo, mismo formato por bloques
  undo/                 el historial que hace posible deshacer
```

Los guardados reescriben solo los bloques que efectivamente cambiaron,
y nunca ocurren mientras la cinta está rodando (REQ-802, si estás
leyendo la especificación directamente).

## Compilarlo vos mismo

```bash
cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

Ese es todo el control de calidad: formato, análisis estático con las
advertencias tratadas como errores, y la suite de pruebas completa. Es
lo mismo que corre CI en cada commit y lo mismo que tiene que estar en
verde antes de que cualquier cambio entre. En una máquina sin toolchain
de Rust, un script envoltorio de Docker corre los comandos idénticos
dentro de un contenedor.
