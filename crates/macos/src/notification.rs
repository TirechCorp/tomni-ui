//! As notificações do sistema do macOS, pelo `UNUserNotificationCenter` (o
//! `NSUserNotification` antigo não mostra nada nas versões novas).
//!
//! O centro só existe para um app empacotado (`.app` com `CFBundleIdentifier`): fora dele o
//! `currentNotificationCenter` lança uma exceção que derruba o processo, então
//! [`Notifications::new`] devolve `None` antes de chamá-lo (o `cargo run`). Na primeira vez o
//! sistema pergunta se o app pode notificar; a resposta fica nos Ajustes do Sistema.
//!
//! O clique chega ao delegate (`didReceiveNotificationResponse`), numa thread do sistema, com o
//! número da notificação guardado no `userInfo`. O delegate também diz para mostrar a
//! notificação com o app em primeiro plano (`willPresentNotification`), que o sistema esconde
//! por padrão.

use std::cell::RefCell;
use std::collections::VecDeque;

use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, NSObject, NSObjectProtocol};
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_foundation::{NSArray, NSBundle, NSNumber, NSString};

#[link(name = "UserNotifications", kind = "framework")]
unsafe extern "C" {}

/// Quantas notificações ficam guardadas para o [`Notifications::close`].
const KEPT: usize = 64;

/// A chave do `userInfo` com o número da notificação.
const ID_KEY: &str = "tomni-ui-id";

/// `UNAuthorizationOptionSound | UNAuthorizationOptionAlert`.
const AUTHORIZATION: usize = (1 << 1) | (1 << 2);

/// `UNNotificationPresentationOptionSound | List | Banner`: com o app em primeiro plano, a
/// notificação aparece como apareceria com ele atrás.
const PRESENTATION: usize = (1 << 1) | (1 << 3) | (1 << 4);

/// O que o delegate guarda: quem avisar do clique.
struct Ivars {
    clicked: Box<dyn Fn(u64) + Send + Sync>,
}

define_class!(
    // SAFETY: o `NSObject` não tem exigência para subclasses, e o `Delegate` não implementa
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[ivars = Ivars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    impl Delegate {
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: &AnyObject,
            response: &AnyObject,
            handler: &DynBlock<dyn Fn()>,
        ) {
            if let Some(id) = clicked_id(response) {
                (self.ivars().clicked)(id);
            }
            handler.call(());
        }

        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &AnyObject,
            _notification: &AnyObject,
            handler: &DynBlock<dyn Fn(usize)>,
        ) {
            handler.call((PRESENTATION,));
        }
    }
);

impl Delegate {
    fn new(clicked: impl Fn(u64) + Send + Sync + 'static) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Ivars {
            clicked: Box::new(clicked),
        });
        // SAFETY: o `init` do `NSObject` é o inicializador designado da superclasse.
        unsafe { msg_send![super(this), init] }
    }
}

/// O número guardado no `userInfo` da notificação que o usuário clicou.
fn clicked_id(response: &AnyObject) -> Option<u64> {
    // SAFETY: `response` é o `UNNotificationResponse` que o sistema passou ao delegate, e
    // `notification` é uma propriedade dele.
    let notification: Option<Retained<AnyObject>> = unsafe { msg_send![response, notification] };
    // SAFETY: `notification` é um `UNNotification`, e `request` é uma propriedade dele.
    let request: Option<Retained<AnyObject>> = unsafe { msg_send![&*notification?, request] };
    // SAFETY: `request` é um `UNNotificationRequest`, e `content` é uma propriedade dele.
    let content: Option<Retained<AnyObject>> = unsafe { msg_send![&*request?, content] };
    // SAFETY: `content` é um `UNNotificationContent`, e `userInfo` é um `NSDictionary`.
    let info: Option<Retained<AnyObject>> = unsafe { msg_send![&*content?, userInfo] };
    let key = NSString::from_str(ID_KEY);
    // SAFETY: `info` é um `NSDictionary`, e a chave é um `NSString`.
    let number: Option<Retained<AnyObject>> = unsafe { msg_send![&*info?, objectForKey: &*key] };
    let number = number?.downcast::<NSNumber>().ok()?;
    Some(number.unsignedLongLongValue())
}

/// Se o processo é um app empacotado: sem isso o centro de notificações derruba o processo.
fn in_bundle() -> bool {
    let bundle = NSBundle::mainBundle();
    bundle.bundleIdentifier().is_some() && bundle.bundlePath().to_string().ends_with(".app")
}

/// Quem mostra as notificações do app.
pub struct Notifications {
    center: Retained<AnyObject>,
    /// O centro guarda o delegate só por referência fraca.
    _delegate: Retained<Delegate>,
    /// O número e o identificador de cada notificação mostrada, para o [`Notifications::close`].
    shown: RefCell<VecDeque<(u64, Retained<NSString>)>>,
}

impl std::fmt::Debug for Notifications {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Notifications")
            .field("shown", &self.shown.borrow().len())
            .finish_non_exhaustive()
    }
}

impl Notifications {
    /// Prepara as notificações e pede a permissão ao sistema (a pergunta só aparece na primeira
    /// vez). `clicked` recebe o número da notificação clicada, numa thread do sistema. `None`
    /// fora de um `.app`.
    pub fn new(clicked: impl Fn(u64) + Send + Sync + 'static) -> Option<Self> {
        if !in_bundle() {
            return None;
        }
        let class = AnyClass::get(c"UNUserNotificationCenter")?;
        // SAFETY: o método de classe devolve o centro do app; o processo está num `.app`
        // (conferido acima), sem o que ele lançaria uma exceção.
        let center: Option<Retained<AnyObject>> =
            unsafe { msg_send![class, currentNotificationCenter] };
        let center = center?;
        let delegate = Delegate::new(clicked);
        // SAFETY: o delegate responde aos dois métodos do `UNUserNotificationCenterDelegate` e
        // fica vivo em `_delegate` enquanto o centro o usa.
        let () = unsafe { msg_send![&*center, setDelegate: &*delegate] };
        let done = RcBlock::new(|_granted: Bool, _error: *mut AnyObject| {});
        // SAFETY: as opções são as do `UNAuthorizationOptions`, e o bloco tem a assinatura
        // `void (^)(BOOL, NSError *)` que o método pede.
        let () = unsafe {
            msg_send![&*center, requestAuthorizationWithOptions: AUTHORIZATION, completionHandler: &*done]
        };
        Some(Self {
            center,
            _delegate: delegate,
            shown: RefCell::new(VecDeque::new()),
        })
    }

    /// Mostra a notificação `id`. A de mesma `tag` toma o lugar da anterior.
    pub fn show(&self, id: u64, title: &str, body: &str, tag: Option<&str>) {
        let _ = self.try_show(id, title, body, tag);
    }

    fn try_show(&self, id: u64, title: &str, body: &str, tag: Option<&str>) -> Option<()> {
        let content_class = AnyClass::get(c"UNMutableNotificationContent")?;
        let request_class = AnyClass::get(c"UNNotificationRequest")?;
        let sound_class = AnyClass::get(c"UNNotificationSound")?;
        let dictionary_class = AnyClass::get(c"NSDictionary")?;
        // SAFETY: `new` de uma classe que herda do `NSObject`.
        let content: Option<Retained<AnyObject>> = unsafe { msg_send![content_class, new] };
        let content = content?;
        let title = NSString::from_str(title);
        // SAFETY: `title` é uma propriedade `NSString` do `UNMutableNotificationContent`.
        let () = unsafe { msg_send![&*content, setTitle: &*title] };
        if !body.is_empty() {
            let body = NSString::from_str(body);
            // SAFETY: `body` é uma propriedade `NSString` do `UNMutableNotificationContent`.
            let () = unsafe { msg_send![&*content, setBody: &*body] };
        }
        // SAFETY: `defaultSound` é um método de classe do `UNNotificationSound`.
        let sound: Option<Retained<AnyObject>> = unsafe { msg_send![sound_class, defaultSound] };
        if let Some(sound) = sound {
            // SAFETY: `sound` é uma propriedade `UNNotificationSound` do conteúdo.
            let () = unsafe { msg_send![&*content, setSound: &*sound] };
        }
        let number = NSNumber::numberWithUnsignedLongLong(id);
        let key = NSString::from_str(ID_KEY);
        // SAFETY: `dictionaryWithObject:forKey:` recebe dois objetos, e a chave é copiável.
        let info: Option<Retained<AnyObject>> =
            unsafe { msg_send![dictionary_class, dictionaryWithObject: &*number, forKey: &*key] };
        // SAFETY: `userInfo` é uma propriedade `NSDictionary` do conteúdo.
        let () = unsafe { msg_send![&*content, setUserInfo: &*info?] };
        let identifier = NSString::from_str(&match tag {
            Some(tag) => format!("tomni-ui-tag-{tag}"),
            None => format!("tomni-ui-{id}"),
        });
        // SAFETY: o método de classe recebe um `NSString`, o conteúdo e um gatilho nulo (mostrar
        // já).
        let request: Option<Retained<AnyObject>> = unsafe {
            msg_send![
                request_class,
                requestWithIdentifier: &*identifier,
                content: &*content,
                trigger: std::ptr::null::<AnyObject>()
            ]
        };
        let none: Option<&DynBlock<dyn Fn(*mut AnyObject)>> = None;
        // SAFETY: o pedido é um `UNNotificationRequest`, e o bloco do fim pode ser nulo.
        let () = unsafe {
            msg_send![&*self.center, addNotificationRequest: &*request?, withCompletionHandler: none]
        };
        let mut shown = self.shown.borrow_mut();
        shown.retain(|(_, old)| **old != *identifier);
        shown.push_back((id, identifier));
        while shown.len() > KEPT {
            shown.pop_front();
        }
        Some(())
    }

    /// Tira a notificação `id` da tela e da Central de Notificações.
    pub fn close(&self, id: u64) {
        let identifier = {
            let mut shown = self.shown.borrow_mut();
            let Some(at) = shown.iter().position(|(shown, _)| *shown == id) else {
                return;
            };
            shown.remove(at).map(|(_, identifier)| identifier)
        };
        let Some(identifier) = identifier else {
            return;
        };
        let identifiers = NSArray::from_slice(&[&*identifier]);
        // SAFETY: o método recebe um `NSArray` de `NSString`.
        let () = unsafe {
            msg_send![&*self.center, removeDeliveredNotificationsWithIdentifiers: &*identifiers]
        };
        // SAFETY: o método recebe um `NSArray` de `NSString`.
        let () = unsafe {
            msg_send![&*self.center, removePendingNotificationRequestsWithIdentifiers: &*identifiers]
        };
    }
}
