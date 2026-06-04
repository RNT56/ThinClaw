pub const CHATML_TEMPLATE: &str = r#"{% for message in messages %}{{'<|im_start|>' + message['role'] + '\n' + message['content'] + '<|im_end|>' + '\n'}}{% endfor %}{% if add_generation_prompt %}{{ '<|im_start|>assistant\n' }}{% endif %}"#;

pub const LLAMA3_TEMPLATE: &str = r#"{% set loop_messages = messages %}{% for message in loop_messages %}{% set content = '<|start_header_id|>' + message['role'] + '<|end_header_id|>\n\n'+ message['content'] | trim + '<|eot_id|>' %}{% if loop.index0 == 0 %}{% set content = '<|begin_of_text|>' + content %}{% endif %}{{ content }}{% endfor %}{% if add_generation_prompt %}{{ '<|start_header_id|>assistant<|end_header_id|>\n\n' }}{% endif %}"#;

pub const MISTRAL_TEMPLATE: &str = r#"{% for message in messages %}{% if message['role'] == 'user' %}{{ '[INST] ' + message['content'] + ' [/INST]' }}{% elif message['role'] == 'assistant' %}{{ message['content'] }}{% endif %}{% endfor %}"#;

pub const GEMMA_TEMPLATE: &str = r#"{% for message in messages %}{% if message['role'] == 'assistant' %}{{'<start_of_turn>model\n' + message['content'] + '<end_of_turn>\n'}}{% else %}{{'<start_of_turn>' + message['role'] + '\n' + message['content'] + '<end_of_turn>\n'}}{% endif %}{% endfor %}{% if add_generation_prompt %}{{ '<start_of_turn>model\n' }}{% endif %}"#;

pub const QWEN_TEMPLATE: &str = r#"{% for message in messages %}{{'<|im_start|>' + message['role'] + '\n' + message['content'] + '<|im_end|>' + '\n'}}{% endfor %}{% if add_generation_prompt %}{{ '<|im_start|>assistant\n' }}{% endif %}"#;
