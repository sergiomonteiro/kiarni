# ADR 0022 — Contagem limitada de documentos por coleção

Status: aceito para alpha local. Data: 2026-11-13. Marco M3/RF-08.

## Contexto e decisão

O RF-08 admite contagem aproximada/exata conforme contrato. O formato 1 não
mantém totais por coleção. Adicionar um contador persistido exigiria mudar o
formato, migrar bases e modificar todos os caminhos de escrita. Uma varredura
sem limite prejudicaria a previsibilidade. Neste incremento, ler o intervalo
do índice primário existente em um único snapshot, sem copiar ou interpretar
JSON e sem alterar escritas, persistência ou publicação de eventos.

Adicionar GET/HEAD `/v1/collections/{collection}/count`, autenticado, sem corpo
ou query, com controles HTTP existentes. Resposta:

```json
{"count":"30","exact":true,"limits":{"documents":10000,"document_bytes":8388608}}
```

`count` é string decimal. `exact:true` significa cardinalidade exata daquele
snapshot; `exact:false` é limite inferior, exibido como **pelo menos N**, não
estimativa estatística. Contar no máximo 10.000 entradas/8 MiB de bytes de
valores. Uma entrada adicional é consultada para distinguir fim do intervalo de
truncamento; ela não entra no total quando excede qualquer orçamento. Isso
limita inspeção a 10.001 entradas e bytes consultados a 8 MiB mais um documento
(até 1 MiB no formato atual), além de páginas/índice/cache do engine. Não é uma
promessa de RSS ou latência física. Valor acima do teto de formato é erro,
não contagem parcial bem-sucedida.

Catálogo/geração da coleção e intervalo são conferidos no mesmo snapshot.
Contam-se entradas do índice, sem revalidar o JSON: não é verificação de
integridade. Inserções e exclusões confirmadas são refletidas na leitura
seguinte; erro, duplicata e no-op não alteram cardinalidade. O snapshot não é
atômico com consultas ou feeds feitos em outras requisições.

Compartilhar o gate FIFO das consultas: no máximo uma query/contagem ativa,
espera de admissão de até 100 ms. Não bloquear writers com novo lock. A leitura
ativa termina no orçamento; falha de storage retorna erro sem valor parcial.
Não prometer cancelamento imediato de I/O físico.

No dashboard, coleta explícita para a coleção selecionada, independente do
resultado limitado da consulta. Mostrar `exact` ou limite inferior, sem polling,
histórico ou atualização automática por eventos. Troca de coleção, fechamento,
atualização geral, desconexão e saída cancelam/inibem resultados antigos. Erros
limpam o valor; 401 limpa a sessão. Não disparar uma contagem por coleção na lista.

## Consequências

Sem contador derivado, migração, dependência nova ou alteração do formato 1.
Coleções grandes podem mostrar somente um limite inferior; contar tudo com
custo constante exigirá outro incremento e evidência específica. Mantêm-se
limites e contrato de queries, e nenhuma garantia nova de changefeeds.

## Evidências

[Verificações locais](../DOCUMENT_COUNTS.md): 241 testes Rust e Chromium real
passaram. Os limites de quantidade/bytes e o erro de layout foram exercitados
no storage; HTTP demonstrou CRUD/no-op, autenticação e restart. O navegador
mostrou contagem independente de query limitada e stream, com cancelamento e
limite inferior explícito. Sem benchmark novo ou alteração do formato 1.
