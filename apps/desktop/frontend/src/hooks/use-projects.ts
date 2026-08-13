import { useState, useEffect, useCallback } from 'react';
import type { Project } from '../lib/bindings';
import { commandClient } from '../lib/command-client';
import { toast } from 'sonner';

export function useProjects() {
    const [projects, setProjects] = useState<Project[]>([]);
    const [isLoading, setIsLoading] = useState(false);

    const fetchProjects = useCallback(async () => {
        try {
            setIsLoading(true);
            setProjects(await commandClient.listProjects());
        } catch (error) {
            console.error('Failed to fetch projects', error);
            toast.error('Failed to load projects');
        } finally {
            setIsLoading(false);
        }
    }, []);

    const createProject = async (name: string, description: string | null) => {
        try {
            const project = await commandClient.createProject({ name, description });
            setProjects(prev => [project, ...prev]);
            toast.success('Project created');
            return project;
        } catch (error) {
            console.error('Failed to create project', error);
            toast.error('Failed to create project');
            throw error;
        }
    };

    const deleteProject = async (id: string) => {
        try {
            await commandClient.deleteProject(id);
            setProjects(prev => prev.filter(p => p.id !== id));
            toast.success('Project deleted');
        } catch (error) {
            console.error('Failed to delete project', error);
            toast.error('Failed to delete project');
        }
    };

    const updateProjectsOrder = async (orders: [string, number][]) => {
        try {
            await commandClient.updateProjectsOrder(orders);
            // Optimistically update local state if needed, or just let the caller handle it
        } catch (error) {
            console.error('Failed to update project order', error);
            toast.error('Failed to save project order');
        }
    };

    useEffect(() => {
        fetchProjects();
    }, [fetchProjects]);

    return {
        projects,
        isLoading,
        fetchProjects,
        createProject,
        deleteProject,
        updateProjectsOrder
    };
}
