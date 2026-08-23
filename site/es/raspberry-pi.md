---
title: Configuración de Raspberry Pi
description: Lo que costó convertir openporta en un instrumento real y dedicado sobre una Raspberry Pi 4 - modo kiosco, hardware real y las asperezas que vinieron con ambos.
date: 2026-08-22
lang: es
---

*[English](../raspberry-pi.html) · **Español***

# Configuración de Raspberry Pi

El motor y la interfaz no saben que corren sobre nada en particular,
pero hacer que una Raspberry Pi 4 realmente *se sienta* como un
instrumento dedicado, en vez de un escritorio Linux que casualmente
corre esta aplicación, requirió trabajo real más allá del camino de
audio en sí. Esta es esa historia.

![El escritorio, con un ícono lanzador con forma de casete](../images/desktop-icon.png)

## El camino del hardware

El audio en tiempo real pasa por cpal contra una interfaz USB
compatible con la clase estándar; las pruebas de este proyecto se
hicieron contra una Zoom L6. Conseguir una conexión limpia y repetible
implicó resolver un problema que en realidad no era de audio: ALSA
enumera una sola interfaz física muchas veces (`hw:`, `plughw:`,
`dmix`, `front`, ...), todas con nombres de pantalla casi idénticos y
sin forma confiable de distinguirlas solo por el nombre. El host propio
de PipeWire habla con el mismo hardware como un único dispositivo real,
y eso fue lo que hizo utilizable la selección por nombre en esta
plataforma.

Una vez elegido un dispositivo, la aplicación lo recuerda:
`~/.config/openporta/` guarda el último dispositivo de entrada/salida,
el período y el mapa de canales de entrada por pista que se conectaron
correctamente, y la aplicación vuelve a intentar esa combinación
automáticamente en cada arranque. La idea es que encienda lista, como
lo hace un equipo real, y no que se quede inactiva esperando que le
digan qué enchufar.

## Modo kiosco

`--kiosk` elimina todo rastro de decoración de ventana y toma la
pantalla completa, lanzado automáticamente al iniciar sesión mediante
una entrada de autoarranque por usuario: no se toca nada en `/etc`, así
que sobrevive a una actualización del sistema y no necesita `sudo`.
Escape permite salir desde el teclado; matar el proceso por ssh siempre
funciona sin importar qué tenga el foco localmente, algo que importa
más de lo que parece la primera vez que una ventana en kiosco no
responde a nada más.

Junto a la entrada de autoarranque existen un lanzador en la barra de
tareas y un ícono de escritorio con forma de casete dibujado a mano,
para abrirlo manualmente en lugar de esperar a un reinicio: en modo
ventana, no kiosco, porque un lanzamiento manual normalmente quiere
seguir llegando al resto del escritorio.

### El teclado, y el panel que rompió

El modo kiosco usaba originalmente el estado de pantalla completa real
del compositor, no solo una ventana maximizada sin bordes: visualmente
idénticos, pero resultó importar muchísimo para qué otra cosa puede
dibujarse en pantalla al mismo tiempo. Un teclado en pantalla
(`wvkbd`, elegido porque está hecho justamente para este tipo de
compositor wlroots en vez de necesitar X11) resultaba completamente
invisible detrás de una ventana en pantalla completa real: el protocolo
layer-shell de Wayland define la capa del teclado *por debajo* de una
superficie de pantalla completa exclusiva, por diseño y no por error.

La solución fue dejar de pedir ese estado exclusivo y usar una ventana
maximizada sin bordes: idéntica a la vista, pero ya no lo bastante
arriba en la pila de superficies como para tapar un teclado
layer-shell. Eso cambió un problema por otro más chico: las ventanas
maximizadas respetan el espacio reservado del panel del escritorio en
lugar de cubrirlo, así que volvió la barra de tareas. El arreglo final
terminó siendo de dos lados: suprimir el panel específicamente mientras
el modo kiosco está activo (congelando su propio proceso supervisor en
vez de tocar algún archivo del sistema) y restaurarlo apenas termina el
modo kiosco, ya sea porque se cierra la aplicación o simplemente porque
se apretó Escape.

El teclado en sí solo aparece con un interruptor explícito, no
automáticamente cuando un campo de texto recibe el foco, ya que eso
requeriría que la aplicación hable el protocolo text-input de Wayland,
y si el toolkit subyacente realmente lo hace no era algo con lo que
valiera la pena apostar toda la función. El interruptor además revisa
primero: si ya hay un teclado físico real conectado (detectado igual
que lo hace el propio Linux, con `ID_INPUT_KEYBOARD` en el dispositivo
de entrada, no adivinando por una lista de fabricantes), no hace nada.
No hay razón para ofrecer un teclado en pantalla redundante sobre uno
real.

## Qué se verificó en hardware, no solo en teoría

- Grabación y guardado full-duplex contra una interfaz real, con un
  período de 256 muestras
- Reconexión automática al dispositivo sobreviviendo un reinicio real
- Autoarranque en kiosco sobreviviendo un reinicio real
- La vista del mezclador entrando en la pantalla real de 800x480 del
  kiosco sin scroll
- El teclado en pantalla dibujándose correctamente por encima de la
  ventana en kiosco, y el panel del escritorio desapareciendo y
  reapareciendo limpiamente alrededor

Lo que queda abierto: una pasada formal de rendimiento del tiempo de
callback, midiendo el margen real con una mezcla o varias pistas
armadas corriendo simultáneamente a un período de 128-256 muestras, en
vez de suponer que entra.

## Todo lo demás en esta Pi

Patchbox OS viene con Pure Data, SuperCollider, Audacity, Patchage y
una prueba de Pianoteq ya en el escritorio: openporta corre junto a
todo eso como un ícono más, no como un reemplazo del resto del
escritorio de software de audio que ya está ahí.
