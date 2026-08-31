# Changelog del add-on

Cada versión publicada aparece aquí con un resumen de lo que cambia para quien usa FutureFin como
add-on de Home Assistant. El detalle técnico completo — cifras exactas, issues y el porqué de cada
decisión — vive en el
[CHANGELOG del proyecto](https://github.com/maxlainz/FutureFin/blob/main/CHANGELOG.md).

## 4.6.0

- El asistente de IA (chat MCP) deja de recalcular por su cuenta cifras que el servidor ya sabía:
  ahora también avisa cuándo se agota el patrimonio proyectado, qué parte del gasto queda sin
  cubrir por ningún activo, y si una hipoteca con cuota insuficiente está creciendo en vez de
  amortizarse. Antes esos casos podían pasar desapercibidos.
- La pantalla de Jubilación deja de recalcular «a ojo» el primer cruce con tu objetivo FIRE y lee
  la cifra exacta del servidor — en instalaciones con tramos altos de ahorro podía llegar a
  mostrar un objetivo hasta un 20 % distinto del real.
- Si ya has cruzado tu objetivo, el gráfico de patrimonio muestra el horizonte completo en vez de
  cortarlo a los 12 meses siguientes al cruce.
- Varias fichas de ayuda («i») de la app quedan más completas y precisas.

## 4.5.0

- Guardar `null` en un campo desde la API o desde una herramienta del asistente de IA ahora lo
  borra de verdad — en seis campos concretos, el servidor respondía «hecho» sin cambiar nada.
- El techo del tope de una regla de reparto («hasta X €/mes») se calcula siempre, también en
  meses sin caja sobrante o ya jubilado.
- Corregido un fallo donde un tipo de interés tecleado con coma en vez de punto (por ejemplo
  «3,50» leído como 350) podía colarse sin aviso; el máximo admitido pasa a ser 100 %/año.
- Corregido un fallo en el formulario de escalado del ahorro por tramos: un umbral tecleado como
  «6.000 €» podía interpretarse como 6 € y disparar el objetivo FIRE calculado hasta un 13 % por
  encima del real, en silencio.
- Una rentabilidad esperada absurdamente alta ya no puede tumbar la proyección con un error
  interno; ahora se rechaza con un mensaje claro.
- Varias correcciones de coherencia en el asistente de IA (nombres de parámetro, permisos,
  validación de aportes).

## 4.4.1

- Ronda de correcciones sin cambios de comportamiento visible: mensajes de error más precisos y
  documentación del asistente de IA (MCP) alineada con lo que el servidor hace de verdad.

## 4.4.0

- El asistente de IA (MCP) pasa de 52 a 68 herramientas: ahora puede decirte cuánto pagas de
  intereses en cada deuda y cuándo terminas de pagarla, simular si compensa amortizar antes,
  sumar gastos filtrados sin tener que contarlos a mano, mostrar el patrimonio en euros de hoy, y
  anotar movimientos pasados directamente desde la conversación.
- Varias cifras que el asistente podía enseñar mal quedan corregidas: el patrimonio histórico, el
  objetivo FIRE del mes exacto de jubilación, y los meses sin movimientos (que se leían como gasto
  cero en vez de «sin datos»).
- Si dejas correr el asistente sin supervisión: cada escritura queda auditada (quién, con qué
  credencial, qué cambió), se pueden emitir credenciales de solo lectura, y las acciones
  irreversibles piden confirmación en dos pasos antes de ejecutarse.

## 4.3.1

- Nueva opción `ha_sso_url`: habilita el botón «Entrar con Home Assistant» al
  abrir FutureFin **fuera del panel** (puerto directo o túnel). Vacía = apagado.
- Con ella, autorizar el conector MCP/OAuth ya no exige una segunda cuenta con
  contraseña. FutureFin no guarda ninguna credencial de Home Assistant.

## 4.3.0

- Primera versión de FutureFin como add-on de Home Assistant.
- Panel en la barra lateral vía ingress.
- Inicio de sesión con la identidad de Home Assistant (opción `sso`).
- Puerto directo opcional (deshabilitado por defecto) para MCP y OAuth, que no
  pueden funcionar a través del ingress.
