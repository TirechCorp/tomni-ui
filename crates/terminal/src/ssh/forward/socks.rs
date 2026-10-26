//! Um servidor SOCKS5 mínimo para o `-D`: só o RFC 1928, só o comando CONNECT e sem
//! autenticação. O destino é entregue como texto (um nome vai ao servidor SSH sem ser
//! resolvido aqui, o `socks5h` dos navegadores).

use std::net::{Ipv4Addr, Ipv6Addr};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// A resposta "conseguiu".
pub(super) const SUCCEEDED: u8 = 0x00;
/// Falha geral do servidor SOCKS.
pub(super) const GENERAL_FAILURE: u8 = 0x01;
/// Rede ou máquina inalcançável (o tempo esgotou).
pub(super) const HOST_UNREACHABLE: u8 = 0x04;
/// Conexão recusada.
pub(super) const REFUSED: u8 = 0x05;
const COMMAND_NOT_SUPPORTED: u8 = 0x07;
const ADDRESS_NOT_SUPPORTED: u8 = 0x08;

/// O pedido não pôde seguir; a conexão é fechada (a resposta, se o protocolo pede uma, já foi
/// escrita).
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Rejected;

async fn read_byte<S: AsyncRead + Unpin>(stream: &mut S) -> Result<u8, Rejected> {
    stream.read_u8().await.map_err(|_| Rejected)
}

/// Responde ao pedido com `code` (o endereço de volta é sempre `0.0.0.0:0`).
pub(super) async fn reply<S: AsyncWrite + Unpin>(stream: &mut S, code: u8) {
    let _ = stream.write_all(&[5, code, 0, 1, 0, 0, 0, 0, 0, 0]).await;
    let _ = stream.flush().await;
}

/// A saudação e o pedido: devolve o destino (`host`, `porta`) de um CONNECT.
pub(super) async fn handshake<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
) -> Result<(String, u16), Rejected> {
    if read_byte(stream).await? != 5 {
        return Err(Rejected);
    }
    let count = usize::from(read_byte(stream).await?);
    let mut methods = vec![0u8; count];
    stream
        .read_exact(&mut methods)
        .await
        .map_err(|_| Rejected)?;
    if !methods.contains(&0) {
        let _ = stream.write_all(&[5, 0xFF]).await;
        return Err(Rejected);
    }
    stream.write_all(&[5, 0]).await.map_err(|_| Rejected)?;

    let mut head = [0u8; 4];
    stream.read_exact(&mut head).await.map_err(|_| Rejected)?;
    let [version, command, _, kind] = head;
    if version != 5 {
        return Err(Rejected);
    }
    if command != 1 {
        reply(stream, COMMAND_NOT_SUPPORTED).await;
        return Err(Rejected);
    }
    let host = match kind {
        1 => {
            let mut octets = [0u8; 4];
            stream.read_exact(&mut octets).await.map_err(|_| Rejected)?;
            Ipv4Addr::from(octets).to_string()
        }
        3 => {
            let len = usize::from(read_byte(stream).await?);
            let mut name = vec![0u8; len];
            stream.read_exact(&mut name).await.map_err(|_| Rejected)?;
            String::from_utf8(name).map_err(|_| Rejected)?
        }
        4 => {
            let mut octets = [0u8; 16];
            stream.read_exact(&mut octets).await.map_err(|_| Rejected)?;
            Ipv6Addr::from(octets).to_string()
        }
        _ => {
            reply(stream, ADDRESS_NOT_SUPPORTED).await;
            return Err(Rejected);
        }
    };
    let port = stream.read_u16().await.map_err(|_| Rejected)?;
    Ok((host, port))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    /// Roda o `handshake` contra `bytes` e devolve o resultado e o que ele respondeu.
    async fn rodar(bytes: &[u8]) -> (Result<(String, u16), Rejected>, Vec<u8>) {
        let (mut cliente, mut servidor) = duplex(1024);
        cliente.write_all(bytes).await.unwrap();
        cliente.shutdown().await.unwrap();
        let resultado = handshake(&mut servidor).await;
        drop(servidor);
        let mut resposta = Vec::new();
        cliente.read_to_end(&mut resposta).await.unwrap();
        (resultado, resposta)
    }

    #[tokio::test]
    async fn connect_com_nome_vai_sem_resolver() {
        let mut pedido = vec![5, 1, 0, 5, 1, 0, 3, 11];
        pedido.extend_from_slice(b"exemplo.com");
        pedido.extend_from_slice(&443u16.to_be_bytes());
        let (resultado, resposta) = rodar(&pedido).await;
        assert_eq!(resultado, Ok(("exemplo.com".to_owned(), 443)));
        assert_eq!(resposta, [5, 0]);
    }

    #[tokio::test]
    async fn connect_com_ipv4_e_ipv6() {
        let mut pedido = vec![5, 2, 2, 0, 5, 1, 0, 1, 10, 0, 0, 7];
        pedido.extend_from_slice(&5432u16.to_be_bytes());
        assert_eq!(rodar(&pedido).await.0, Ok(("10.0.0.7".to_owned(), 5432)));

        let mut pedido = vec![5, 1, 0, 5, 1, 0, 4];
        pedido.extend_from_slice(&Ipv6Addr::LOCALHOST.octets());
        pedido.extend_from_slice(&80u16.to_be_bytes());
        assert_eq!(rodar(&pedido).await.0, Ok(("::1".to_owned(), 80)));
    }

    #[tokio::test]
    async fn comando_bind_recebe_07() {
        let (resultado, resposta) = rodar(&[5, 1, 0, 5, 2, 0, 1, 1, 2, 3, 4, 0, 80]).await;
        assert_eq!(resultado, Err(Rejected));
        assert_eq!(resposta, [5, 0, 5, 7, 0, 1, 0, 0, 0, 0, 0, 0]);
    }

    #[tokio::test]
    async fn tipo_de_endereco_desconhecido_recebe_08() {
        let (resultado, resposta) = rodar(&[5, 1, 0, 5, 1, 0, 9]).await;
        assert_eq!(resultado, Err(Rejected));
        assert_eq!(&resposta[2..4], [5, 8]);
    }

    #[tokio::test]
    async fn so_usuario_e_senha_recebe_ff() {
        let (resultado, resposta) = rodar(&[5, 1, 2]).await;
        assert_eq!(resultado, Err(Rejected));
        assert_eq!(resposta, [5, 0xFF]);
    }

    #[tokio::test]
    async fn outra_versao_fecha_sem_responder() {
        let (resultado, resposta) = rodar(&[4, 1, 0, 80, 1, 2, 3, 4, 0]).await;
        assert_eq!(resultado, Err(Rejected));
        assert!(resposta.is_empty());
    }

    #[tokio::test]
    async fn pedido_cortado_fecha() {
        let (resultado, _) = rodar(&[5, 1, 0, 5, 1, 0, 3, 11, b'a']).await;
        assert_eq!(resultado, Err(Rejected));
    }
}
