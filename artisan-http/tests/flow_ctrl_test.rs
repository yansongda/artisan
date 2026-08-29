use artisan_http::Rocket;
use artisan_http::flow_ctrl::FlowCtrl;
use artisan_http::plugin::Plugin;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

struct TestPlugin {
    name: String,
}

#[async_trait]
impl Plugin for TestPlugin {
    async fn assembly(
        &self,
        rocket: &mut Rocket,
        next: artisan_http::flow_ctrl::Next<'_>,
    ) -> artisan_http::Result<()> {
        rocket
            .payload
            .insert("visited".to_string(), serde_json::json!(self.name.clone()));
        next.call(rocket).await
    }
}

#[tokio::test]
async fn test_flow_ctrl_basic() {
    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(TestPlugin {
            name: "plugin1".to_string(),
        }),
        Arc::new(TestPlugin {
            name: "plugin2".to_string(),
        }),
    ];

    let mut ctrl = FlowCtrl::new(plugins);
    let mut rocket = Rocket::new(HashMap::new());

    ctrl.call_next(&mut rocket).await.unwrap();

    assert!(rocket.payload.contains_key("visited"));
}

#[tokio::test]
async fn test_flow_ctrl_cease() {
    struct CeasePlugin;

    #[async_trait]
    impl Plugin for CeasePlugin {
        async fn assembly(
            &self,
            rocket: &mut Rocket,
            _next: artisan_http::flow_ctrl::Next<'_>,
        ) -> artisan_http::Result<()> {
            rocket
                .payload
                .insert("ceased".to_string(), serde_json::json!(true));
            Ok(())
        }
    }

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(CeasePlugin),
        Arc::new(TestPlugin {
            name: "should_not_run".to_string(),
        }),
    ];

    let mut ctrl = FlowCtrl::new(plugins);
    let mut rocket = Rocket::new(HashMap::new());

    ctrl.call_next(&mut rocket).await.unwrap();

    assert!(rocket.payload.contains_key("ceased"));
    assert!(!rocket.payload.contains_key("visited"));
}

#[test]
fn test_flow_ctrl_has_next() {
    let plugins: Vec<Arc<dyn Plugin>> = vec![Arc::new(TestPlugin {
        name: "p1".to_string(),
    })];

    let ctrl = FlowCtrl::new(plugins);
    assert!(ctrl.has_next());
}

#[test]
fn test_flow_ctrl_no_next() {
    let plugins: Vec<Arc<dyn Plugin>> = vec![];
    let ctrl = FlowCtrl::new(plugins);
    assert!(!ctrl.has_next());
}

#[test]
fn test_flow_ctrl_skip_rest_direct() {
    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(TestPlugin {
            name: "p1".to_string(),
        }),
        Arc::new(TestPlugin {
            name: "p2".to_string(),
        }),
        Arc::new(TestPlugin {
            name: "p3".to_string(),
        }),
    ];

    let mut ctrl = FlowCtrl::new(plugins);
    assert!(ctrl.has_next());
    assert!(!ctrl.is_ceased());

    ctrl.skip_rest();

    assert!(!ctrl.has_next());
    assert!(ctrl.is_ceased());
}

#[test]
fn test_flow_ctrl_skip_rest_directly() {
    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(TestPlugin {
            name: "p1".to_string(),
        }),
        Arc::new(TestPlugin {
            name: "p2".to_string(),
        }),
    ];

    let mut ctrl = FlowCtrl::new(plugins);
    assert!(ctrl.has_next());
    assert!(!ctrl.is_ceased());

    ctrl.skip_rest();

    assert!(!ctrl.has_next());
    assert!(ctrl.is_ceased());
}

#[test]
fn test_flow_ctrl_is_ceased() {
    let plugins: Vec<Arc<dyn Plugin>> = vec![Arc::new(TestPlugin {
        name: "p1".to_string(),
    })];
    let ctrl = FlowCtrl::new(plugins.clone());
    assert!(!ctrl.is_ceased());

    let mut ceased_ctrl = FlowCtrl::new(plugins);
    ceased_ctrl.skip_rest();
    assert!(ceased_ctrl.is_ceased());
}

#[test]
fn test_flow_ctrl_debug() {
    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(TestPlugin {
            name: "p1".to_string(),
        }),
        Arc::new(TestPlugin {
            name: "p2".to_string(),
        }),
    ];

    let ctrl = FlowCtrl::new(plugins);
    let debug_str = format!("{:?}", ctrl);

    assert!(debug_str.contains("cursor"));
    assert!(debug_str.contains("plugins_count"));
    assert!(debug_str.contains("is_ceased"));
}

#[tokio::test]
async fn test_flow_ctrl_empty_plugins() {
    let plugins: Vec<Arc<dyn Plugin>> = vec![];
    let mut ctrl = FlowCtrl::new(plugins);
    let mut rocket = Rocket::new(HashMap::new());

    let result = ctrl.call_next(&mut rocket).await;
    assert!(result.is_ok());
    assert!(rocket.payload.is_empty());
}

#[tokio::test]
async fn test_flow_ctrl_call_next_after_skip_rest() {
    struct MarkPlugin {
        name: String,
    }

    #[async_trait]
    impl Plugin for MarkPlugin {
        async fn assembly(
            &self,
            rocket: &mut Rocket,
            next: artisan_http::flow_ctrl::Next<'_>,
        ) -> artisan_http::Result<()> {
            rocket
                .payload
                .insert(self.name.clone(), serde_json::json!(true));
            next.call(rocket).await
        }
    }

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(MarkPlugin {
            name: "first".to_string(),
        }),
        Arc::new(MarkPlugin {
            name: "second".to_string(),
        }),
    ];

    let mut ctrl = FlowCtrl::new(plugins);
    let mut rocket = Rocket::new(HashMap::new());

    // 先手动调用 skip_rest
    ctrl.skip_rest();

    // 调用 call_next 应该立即返回 Ok(())
    let result = ctrl.call_next(&mut rocket).await;
    assert!(result.is_ok());
    assert!(rocket.payload.is_empty());
}

#[test]
fn test_flow_ctrl_skip_rest_clears_has_next() {
    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(TestPlugin {
            name: "p1".to_string(),
        }),
        Arc::new(TestPlugin {
            name: "p2".to_string(),
        }),
        Arc::new(TestPlugin {
            name: "p3".to_string(),
        }),
    ];

    let mut ctrl = FlowCtrl::new(plugins);

    // 初始状态
    assert!(ctrl.has_next());

    // 执行 skip_rest
    ctrl.skip_rest();

    // skip_rest 后 has_next 应为 false（因为 cursor 被设置为 plugins.len())
    assert!(!ctrl.has_next());
}

#[test]
fn test_flow_ctrl_skip_rest_is_terminal() {
    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(TestPlugin {
            name: "p1".to_string(),
        }),
        Arc::new(TestPlugin {
            name: "p2".to_string(),
        }),
    ];

    let mut ctrl = FlowCtrl::new(plugins);
    ctrl.skip_rest();
    assert!(!ctrl.has_next());
    assert!(ctrl.is_ceased());
}
