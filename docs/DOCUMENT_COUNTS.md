# Contagem de documentos por coleção

Recorte M3/RF-08. GET/HEAD `/v1/collections/{collection}/count` exige a mesma
credencial, Host/Origin e limites HTTP da API, sem query ou corpo. Uma coleção
inexistente retorna 404. HEAD mantém headers e não envia corpo. Exemplo:

```json
{"count":"30","exact":true,"limits":{"documents":10000,"document_bytes":8388608}}
```

`count` é decimal em string. `exact:true` indica total exato em um snapshot da
coleção inteira. `exact:false` indica **pelo menos N documentos**, não estimativa
estatística ou total exato. Contagem e consulta são leituras distintas: `count`
no envelope de query continua significando somente a quantidade retornada,
ainda sujeita ao filtro, projeção e limit da AST.

## Custo e consistência

O formato persistido 1 continua inalterado: não há contador novo para atualizar,
migrar ou recuperar. A contagem lê o intervalo do índice primário em um snapshot
junto do catálogo/geração. Inserções e exclusões confirmadas são refletidas na
leitura seguinte; duplicatas, no-ops e updates não alteram a cardinalidade. A
leitura não copia nem interpreta JSON e não certifica integridade dos documentos.
Exclusão/recriação de uma coleção não reutiliza a geração antiga.

O orçamento é de 10.000 entradas e 8 MiB de bytes de valores, mais uma entrada
de lookahead para distinguir fim do intervalo de truncamento. Se essa entrada
exceder algum limite, a leitura retorna a contagem já acumulada com exact:false.
Com dados válidos, consultam-se no máximo 10.001 entradas e 8 MiB mais um
documento de até 1 MiB, além de custos de páginas/índice/cache do engine.
Valor maior que o teto do formato produz erro, não resultado parcial.

O orçamento não limita tempo de I/O físico ou RSS. Contagens compartilham o gate
FIFO de queries: uma execução ativa, espera de admissão de até 100 ms e fila
limitada pelas vagas HTTP. Não há novo lock bloqueando writers. Não há scan
ilimitado, cache de totais ou uma contagem automática para cada coleção listada.
Não há garantia de cancelamento imediato de leitura já iniciada no servidor.

## Dashboard

Selecione uma coleção e use **Contar documentos**. O painel mostra **Total exato:
N** ou **Pelo menos N documentos**, com explicação do limite. Não depende do
filtro da consulta ou da quantidade de documentos exibida. Não fecha o stream,
não faz polling e não tenta reconciliar o total por eventos. Uma contagem antiga
permanece uma leitura pontual até coletar novamente.

Cada coleta tem espera de dez segundos no navegador. Trocar coleção, fechar o
painel, atualizar a visão geral, desconectar ou sair cancela a leitura e limpa
os dados locais. Respostas antigas não atualizam a seleção nova. Erros limpam o
valor; erros HTTP mostram código/request ID quando disponíveis. 401 limpa toda
a sessão. Com apenas uma vaga HTTP configurada, pare o stream antes de contar.

## Verificação

Testes de storage cobrem coleção vazia, duplicata, escrita não confirmada,
exclusão/recriação, reinício, limite de 10.000 documentos, orçamento de bytes,
isolamento entre coleções e erro de layout. Integração com servidor real cobre
GET/HEAD, acesso negado, Host/Origin, recusa de corpo/query, CRUD/no-op e reinício.
O Chromium diferencia contagem da coleção de resultado limitado, conta durante
um stream real e verifica vazio, limite inferior, formato inválido, 404/request
ID, 401 e resposta antiga após trocar de coleção. Limite inferior no navegador
é injetado; os limites efetivos são testados no storage.

[ADR 0022](./adr/0022-contagem-limitada-de-documentos.md). Esta entrega não
conclui M3/RF-09: apresentação de storage, erros recentes e diagnóstico interno necessário
continuam pendentes. [Storage](./STORAGE_METRICS.md) já tem métricas na API. [HTTP](./HTTP_METRICS.md) e [consultas](./QUERY_METRICS.md) já têm
métricas próprias na API. Não há contador exato de custo constante
para coleções arbitrariamente grandes nem benchmark novo deste endpoint.

Validação local em 2026-11-13: 241 testes Rust passaram (dois workers de fixture
ignorados diretamente e executados pelos testes pais), incluindo quatro novos
casos de storage/HTTP. Passaram também fmt, clippy, integração Chromium real,
fronteira de nove pacotes, build release, sintaxe JS, links locais e
`git diff --check`. CI remoto e outras plataformas não foram executados.
