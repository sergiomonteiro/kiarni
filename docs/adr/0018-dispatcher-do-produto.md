# ADR 0018 — Dispatcher de coleção no produto

- Status: aceita para o recorte local M2/RF-05; integrada ao writer e ao SSE autenticado.
- Base: ADRs aceitos 0004/0005 e coordenador do ADR 0017.

## Primeiro recorte

Criar `kiarni-feeds` em `crates/feeds`, independente de HTTP e storage concreto,
com eventos insert/update/delete, estados before/after validados e compartilhados,
assinaturas de coleção, fila limitada e terminais explícitos. Não ligar SSE ou
publicação ao coordenador neste incremento. Não declarar RF-05 concluído.

O primeiro recorte isolado usou limites do ADR 0004: oito assinaturas e oito itens, cada
estado com até 1 MiB. Handles terminais ocupam vaga até Drop. Overflow sinaliza
fora da fila, descarta backlog ao ser observado e não bloqueia publicação.
Debug não contém coleção, ID ou payload. Nenhuma fila ou worker ilimitado.

## Fronteira de integração com grupos

O dispatcher não é clonável; o futuro worker de escrita será seu único dono.
Reserva de até 32 sequências empresta o dispatcher mutavelmente, verifica
esgotamento antes do futuro commit e impede inscrição/publicação concorrente
por outro dono. Reservar não incrementa sequência. Somente publicação confirmada
consome sequência, local à instância e sem replay. O chamador deve confirmar
commit antes de publicar; o dispatcher não prova ou executa durabilidade.

Reserva não usada pode ser descartada após aborto/no-op sem emitir evento.
Uma publicação iniciada e abandonada sem finish interrompe o dispatcher e as
assinaturas para não indicar continuidade após fanout parcial. Interrupção
explícita também fecha o dispatcher. Shutdown preserva o primeiro terminal.

Assinatura começa sob a mesma propriedade exclusiva que a publicação, sem
snapshot ou histórico. A futura inscrição HTTP passará como barreira pelo
coordenador; não basta chamar o dispatcher em paralelo ao commit.

## Limites da entrega

### Integração ao writer

O produto usa os limites já aceitos de configuração (ADR 0012): até 32 streams
e filas de até 16, em vez dos tetos experimentais 8/8. Não reduzir esses valores
silenciosamente nem recusar configurações antes aceitas. A política de overflow
permanece; o plano total de 512 MiB limita combinações reais. Defaults continuam
8 streams e fila 4. A biblioteca admite o teto teórico; o servidor somente usa
combinações aprovadas pelo plano global.

O próximo recorte captura before/after dentro da transação de updates, preserva
o estado removido na transação de delete e usa o candidato confirmado no insert.
Captura só ocorre para coleções com assinantes ativos. Sem observadores, o
dispatcher pode avançar sequência de uma mudança confirmada sem alocar payload,
mas deve verificar que não há assinante daquela coleção. Inscrição é comando
barreira no mesmo writer, validando coleção existente antes de registrar.

Drop de coleção interrompe suas assinaturas como `interrupted`, sem afetar
outras coleções; recriar o nome exige nova assinatura. Os handles interrompidos
continuam ocupando vagas até Drop. Isso impede misturar gerações de coleção.

Reserva precede commit e publicação precede o próximo comando/resposta. Falha
fatal/resultado incerto interrompe o dispatcher; falha depois de commit não
retorna sucesso nem tenta desfazer o dado. No-ops e rejeições não emitem eventos.
No máximo dois estados de 1 MiB por transição são retidos, dentro das reservas
por requisição e das filas já previstas; payload Arc evita cópias na passagem
storage→dispatcher. SSE permanece desativado neste recorte.

Payloads ficam em Arc; o transporte futuro deve limitar eventos em processamento
e cópias de frames. O orçamento das filas não inclui retenção arbitrária pelo
chamador da biblioteca. Before/after de cada transição do grupo, mutações de
inserção/exclusão e reserva de buffers ainda exigem integração própria no storage
e coordenador antes de ativar changefeeds. Não emitir eventos antes dessa etapa.

Este desenho não altera SSE, autenticação, formato persistido, garantias de
entrega best-effort ou o escopo single-node do MVP.

## Evidência do primeiro incremento

O crate de produto usa kiarni-core e std, sem novas dependências externas.
Sete testes cobrem contratos, consumidores em threads distintas e terminais.
Fmt, clippy e 208 testes do workspace passaram, com dois workers de fixture
ignorados diretamente e usados pelos testes pais. A fronteira de dependências
passou para nove pacotes. [Contrato](../../crates/feeds/README.md).

Não foi ligado a mutações ou rede. publish_confirmed exige confirmação prévia
do chamador. Drop antes do primeiro evento não distingue aborto de falha após
commit; a integração deverá chamar interrupt explicitamente nesse último caso.

## Integração implementada — 2026-11-13

O worker é dono do dispatcher. Inscrição entra na fila como barreira e verifica
coleção existente. Updates capturam estados na transação; delete devolve o estado
removido somente após commit; insert usa o candidato confirmado. Reservas de
sequência precedem a mutação e publicação precede o próximo comando e a resposta.
Não há evento para no-op, rejeição ou mutação que não confirmou.

Captura ocorre apenas em coleções observadas. PreparedWrite conserva Vec para
updates não observados e compartilha Arc do resultado quando observado; a mesma
imagem after serve ao recibo e ao evento. Before também usa Arc. Por requisição,
corpo, before, after compartilhado e workspace de validação permanecem dentro
das reservas explícitas de 3 MiB mais resultado de ao menos 1 MiB. Filas usam
a reserva de streams já prevista; cópias de frames/in-flight exigirão revisão
antes de ativar SSE. Isso não constitui limite superior de RSS do allocator.

Falha fatal/resultado incerto interrompe assinaturas e fecha escritas. Um teste
injeta erro após commit e antes do primeiro evento: dado permanece, cliente não
recebe sucesso e observadores recebem interrupted. Pânico do worker também
interrompe, inclusive antes de qualquer publicação. Shutdown conclui grupos
iniciados e fecha feeds, sem drenar backlog. A sequência local reinicia no
processo, sem replay ou atomicidade persistência/publicação.

Doze testes novos cobrem o recorte: captura transacional/deleção, compartilhamento,
limites de produto, configuração acima do spike e sete cenários do coordenador.
Passaram 220 testes, fmt, clippy e a fronteira de nove pacotes. O servidor ainda
não aceita assinaturas pela rede. A próxima etapa é SSE com autenticação e
validação black-box do stream; não foi declarado RF-05 completo.

## Transporte ativo

O transporte SSE do produto foi integrado conforme o ADR 0005. O marco atual
permite experimentar a alpha reativa local, com testes de clientes TCP reais.
Isso não declara o M2 completo: métricas de feeds e validações ampliadas ainda
precisam ser entregues. O endpoint não oferece snapshot inicial ou replay.
A evidência local atual é de 229 testes. [Guia](../ALPHA_LOCAL.md).
