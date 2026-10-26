//! O mundo (planet e moons) contra os arquivos de verdade: o `planet` que o ZeroTier oficial
//! escreve (`planet/terra.planet`) e a moon gerada pelo `zerotier-idtool genmoon` em
//! `testes/zerotier/capturar.sh`.

use std::fs;
use std::path::PathBuf;

use tomni_ui_zerotier::world::{ID_TERRA, Root};
use tomni_ui_zerotier::{Identity, World, WorldKind};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn planet() -> Vec<u8> {
    fs::read(crate_dir().join("planet/terra.planet")).unwrap()
}

fn moon() -> Vec<u8> {
    fs::read(crate_dir().join("tests/fixtures/captura/000000ed158961d1.moon")).unwrap()
}

fn identidade(n: u32) -> Identity {
    let texto =
        fs::read_to_string(crate_dir().join(format!("tests/fixtures/identidades/id{n}.secret")))
            .unwrap();
    Identity::from_secret_str(texto.trim()).unwrap()
}

#[test]
fn o_planet_oficial_e_lido_e_volta_igual() {
    let bytes = planet();
    let mundo = World::parse(&bytes).unwrap();
    assert_eq!(mundo.kind(), WorldKind::Planet);
    assert_eq!(mundo.id(), ID_TERRA);
    assert_eq!(mundo.roots().len(), 4);
    for raiz in mundo.roots() {
        assert!(raiz.identity.validate(), "{}", raiz.identity.address());
        assert!(
            raiz.endpoints
                .iter()
                .any(|e| e.is_ipv4() && e.port() == 9993),
            "toda raiz tem um IPv4 na porta 9993"
        );
    }
    assert_eq!(mundo.to_bytes(), bytes);
}

#[test]
fn o_planet_embutido_e_o_arquivo_do_crate() {
    let terra = World::terra();
    assert_eq!(terra.to_bytes(), planet());
    assert_eq!(terra.id(), ID_TERRA);
}

#[test]
fn a_assinatura_do_planet_confere_com_a_chave_que_ele_mesmo_declara() {
    let mundo = World::parse(&planet()).unwrap();
    assert!(mundo.verify(mundo.updates_signed_by()));
}

#[test]
fn planet_adulterado_nao_confere() {
    let mut bytes = planet();
    // Mexe numa porta de uma raiz.
    let n = bytes.len() - 3;
    bytes[n] ^= 1;
    let mundo = World::parse(&bytes).unwrap();
    assert!(!mundo.verify(mundo.updates_signed_by()));
}

#[test]
fn a_moon_do_idtool_e_lida_e_confere() {
    let bytes = moon();
    let mundo = World::parse(&bytes).unwrap();
    assert_eq!(mundo.kind(), WorldKind::Moon);
    assert_eq!(mundo.id(), identidade(2).address().as_u64());
    assert_eq!(mundo.roots().len(), 1);
    assert_eq!(mundo.roots()[0].identity.address(), identidade(2).address());
    assert_eq!(
        mundo.roots()[0].endpoints,
        vec!["172.31.70.11:9993".parse().unwrap()]
    );
    assert!(mundo.verify(mundo.updates_signed_by()));
    assert_eq!(mundo.to_bytes(), bytes);
}

#[test]
fn mundo_truncado_ou_com_tipo_estranho_e_recusado() {
    let bytes = moon();
    for corte in [0, 1, 17, 100, bytes.len() - 1] {
        assert!(World::parse(&bytes[..corte]).is_err(), "corte em {corte}");
    }
    let mut outro = bytes.clone();
    outro[0] = 9;
    assert!(World::parse(&outro).is_err());
}

#[test]
fn faz_assina_le_e_so_aceita_atualizacao_mais_nova_da_chave_certa() {
    let dono = identidade(1);
    let outra = identidade(3);
    let raiz = Root {
        identity: Identity::from_public_str(&identidade(2).to_public_string()).unwrap(),
        endpoints: vec!["10.0.0.1:9993".parse().unwrap()],
    };
    let v1 = World::make(WorldKind::Moon, 7, 100, &dono, vec![raiz.clone()], &dono).unwrap();
    let v2 = World::make(WorldKind::Moon, 7, 200, &dono, vec![raiz.clone()], &dono).unwrap();
    let falsa = World::make(WorldKind::Moon, 7, 300, &outra, vec![raiz.clone()], &outra).unwrap();
    let outro_id = World::make(WorldKind::Moon, 8, 300, &dono, vec![raiz], &dono).unwrap();

    let relido = World::parse(&v1.to_bytes()).unwrap();
    assert!(relido.verify(&dono.public_bytes().clone()));
    assert!(relido.should_be_replaced_by(&v2));
    assert!(!v2.should_be_replaced_by(&v1), "mais velha não substitui");
    assert!(
        !relido.should_be_replaced_by(&falsa),
        "assinada por outra chave"
    );
    assert!(!relido.should_be_replaced_by(&outro_id), "outro mundo");
}
