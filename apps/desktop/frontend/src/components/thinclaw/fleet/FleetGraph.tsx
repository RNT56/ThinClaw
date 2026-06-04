
import { useEffect } from 'react';
import {
    ReactFlow,
    Background,
    Controls,
    Node,
    Edge,
    useNodesState,
    useEdgesState,
    ConnectionLineType,
    MarkerType
} from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import AgentNode from './AgentNode';

const nodeTypes = {
    agent: AgentNode,
};

interface FleetGraphProps {
    nodes: Node[];
    edges: Edge[];
    onNodeClick: (event: React.MouseEvent, node: Node) => void;
}

export function FleetGraph({ nodes: initialNodes, edges: initialEdges, onNodeClick }: FleetGraphProps) {
    // We lift state up or sync it here.
    // Since we receive nodes from parent based on live data,
    // we want to render what we get but allow ReactFlow to handle internal interactions.
    // However, if parent updates frequently, we need to be careful not to reset user's pan/zoom or node positions if they dragged them.
    // For this "Command Center" view, auto-layout is usually preferred initially.

    // For now, let's just pass them through to a controlled flow or update the hooks.
    // Actually using useNodesState inside here with useEffect sync is a common pattern.

    const [nodes, setNodes, onNodesChange] = useNodesState(initialNodes);
    const [edges, setEdges, onEdgesChange] = useEdgesState(initialEdges);

    useEffect(() => {
        setNodes(initialNodes);
        setEdges(initialEdges);
    }, [initialNodes, initialEdges, setNodes, setEdges]);

    return (
        <div className="w-full h-full bg-[#050505]">
            <ReactFlow
                nodes={nodes}
                edges={edges}
                nodeTypes={nodeTypes}
                onNodesChange={onNodesChange}
                onEdgesChange={onEdgesChange}
                onNodeClick={onNodeClick}
                connectionLineType={ConnectionLineType.SmoothStep}
                fitView
                fitViewOptions={{ padding: 0.2 }}
                minZoom={0.5}
                maxZoom={2}
                defaultEdgeOptions={{
                    type: 'smoothstep',
                    markerEnd: { type: MarkerType.ArrowClosed, color: '#6366f1' },
                    style: { stroke: '#6366f1', strokeWidth: 2, opacity: 0.5 },
                    animated: true,
                }}
            >
                <Background color="#333" gap={20} size={1} />
                <Controls className="!bg-zinc-900 !border-zinc-800 !fill-zinc-400" />
            </ReactFlow>
        </div>
    );
}
